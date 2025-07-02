use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use sqlx::types::Text;
use sqlx::{SqliteConnection, SqlitePool};
use tracing::{info, warn};
use ulid::Ulid;

/// The Vereinsflieger credentials used to access the API.
///
/// These are saved in the `credentials` database table and queried
/// upon startup.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Credentials {
    /// The club ID in Vereinsflieger.
    ///
    /// Go to "Administration" -> "Stammdaten" -> "Verein" and find the
    /// CID value next to the "Kurzname" field.
    pub club_id: u32,

    /// The "Appkey" used to access the Vereinsflieger API.
    ///
    /// This can be generated at "Administration" -> "Stammdaten" -> "Verein"
    /// -> "Einstellungen" -> "REST Interface".
    ///
    /// Appkeys are currently limited to 500 requests per day.
    pub app_key: String,

    /// The username or email address of the account used to access the
    /// Vereinsflieger API.
    pub username: String,

    /// The password of the account used to access the Vereinsflieger API.
    #[sqlx(try_from = "String")]
    pub password: SecretString,
}

impl From<Credentials> for vereinsflieger::Credentials {
    fn from(credentials: Credentials) -> Self {
        Self {
            club_id: Some(credentials.club_id),
            app_key: credentials.app_key.clone(),
            username: credentials.username.clone(),
            password: credentials.password.expose_secret().into(),
            auth_secret: None,
        }
    }
}

impl Credentials {
    /// Find the "first" set of credentials in the database. If multiple
    /// credentials are stored, a random one is returned. In other words,
    /// we expect only one set of credentials to be stored.
    pub async fn find_first(pool: SqlitePool) -> sqlx::Result<Option<Self>> {
        sqlx::query_as(
            r#"
            SELECT club_id, app_key, username, password
            FROM credentials
            "#,
        )
        .fetch_optional(&pool)
        .await
    }

    /// Insert the credentials into the database.
    pub async fn insert(&self, pool: SqlitePool) -> sqlx::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO credentials (club_id, app_key, username, password)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(self.club_id)
        .bind(&self.app_key)
        .bind(&self.username)
        .bind(self.password.expose_secret())
        .execute(&pool)
        .await
        .map(|_| ())
    }
}

/// A member of the club.
///
/// Note that the primary key of the `members` table is the `keycode` field,
/// so a member might exist multiple times in the database if they have
/// multiple keycodes. It is implemented this way to optimize the query
/// performance when looking up a member by their keycode.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Member {
    /// The RFID keycode of the member.
    ///
    /// This currently accepts 10-digit numeric keycodes and 7-digit hexadecimal
    /// keycodes. The latter are converted to the former.
    pub keycode: String,

    /// The member ID inside the club (aka. "Mitgliedsnummer").
    pub id: String,

    /// The first name of the member.
    pub firstname: String,

    /// The last name of the member.
    pub lastname: String,

    /// The nickname of the member. (might be empty)
    pub nickname: String,
}

impl Member {
    /// Find a member by their keycode.
    pub async fn find_by_keycode(pool: SqlitePool, keycode: &str) -> sqlx::Result<Option<Self>> {
        sqlx::query_as(
            r#"
            SELECT keycode, id, firstname, lastname, nickname
            FROM members
            WHERE keycode = $1
            "#,
        )
        .bind(keycode)
        .fetch_optional(&pool)
        .await
    }

    /// Delete all members from the database.
    ///
    /// This should usually be used inside a transaction in combination with
    /// inserting new members.
    async fn delete_all(connection: &mut SqliteConnection) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM members")
            .execute(connection)
            .await
            .map(|_| ())
    }

    /// Insert a member into the database.
    async fn insert(&self, connection: &mut SqliteConnection) -> sqlx::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO members (keycode, id, firstname, lastname, nickname)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(&self.keycode)
        .bind(&self.id)
        .bind(&self.firstname)
        .bind(&self.lastname)
        .bind(&self.nickname)
        .execute(connection)
        .await
        .map(|_| ())
    }

    /// Remove all members from the database and insert a new set of members.
    ///
    /// If any member fails to insert, a warning is logged, but the transaction
    /// is still committed. This ensures that we still insert as many members as
    /// possible, even if some of them e.g. share the same keycode causing a
    /// unique constraint violation.
    pub async fn save_all(pool: SqlitePool, members: Vec<Self>) -> sqlx::Result<()> {
        let mut transaction = pool.begin().await?;

        Self::delete_all(&mut transaction).await?;
        for member in members {
            if let Err(error) = member.insert(&mut transaction).await {
                warn!(
                    "Failed to insert member {} {} with keycode {}: {error}",
                    member.firstname, member.lastname, member.keycode
                );
            }
        }

        transaction.commit().await
    }

    /// Parse a Vereinsflieger keycode into a normalized format.
    ///
    /// This function accepts both the 10-digit numeric format and the 7-digit
    /// hexadecimal format. It returns the 10-digit numeric format.
    pub fn parse_keycode(key: vereinsflieger::Key) -> Option<String> {
        let key = key.name;
        if key.len() == 10 && key.chars().all(|c| c.is_ascii_digit()) {
            Some(key)
        } else if key.len() == 7 && key.chars().all(|c| c.is_ascii_hexdigit()) {
            let key = u32::from_str_radix(&key, 16).ok()?;
            Some(format!("{key:0>10}"))
        } else {
            None
        }
    }
}

/// An article that can be sold in the club.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Article {
    /// The article ID (aka. "Artikelnummer").
    ///
    /// Since there is no dedicated field for barcodes in Vereinsflieger, we
    /// assume that the article ID matches the barcode.
    pub id: String,

    /// The designation of the article (aka. "Bezeichnung").
    pub designation: String,

    /// A mapping of date ranges to prices.
    #[sqlx(json)]
    pub prices: Vec<Price>,
}

impl TryFrom<vereinsflieger::Article> for Article {
    type Error = anyhow::Error;

    fn try_from(article: vereinsflieger::Article) -> Result<Self, Self::Error> {
        Ok(Self {
            id: article.article_id,
            designation: article.designation,
            prices: article
                .prices
                .into_iter()
                .map(Price::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

/// A price for an article that is valid within a certain date range.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Price {
    /// The date from which the price is valid (inclusive).
    pub valid_from: jiff::civil::Date,

    /// The date until which the price is valid (inclusive).
    pub valid_to: jiff::civil::Date,

    /// The unit price of the article.
    pub unit_price: Decimal,
}

impl TryFrom<vereinsflieger::Price> for Price {
    type Error = anyhow::Error;

    fn try_from(price: vereinsflieger::Price) -> Result<Self, Self::Error> {
        Ok(Self {
            valid_from: price.valid_from.parse()?,
            valid_to: price.valid_to.parse()?,
            unit_price: price.unit_price.parse()?,
        })
    }
}

impl Article {
    /// Find an article by its barcode (i.e. article ID).
    pub async fn find_by_barcode(pool: SqlitePool, barcode: &str) -> sqlx::Result<Option<Self>> {
        sqlx::query_as(
            r#"
            SELECT id, designation, prices
            FROM articles
            WHERE id = $1
            "#,
        )
        .bind(barcode)
        .fetch_optional(&pool)
        .await
    }

    /// Delete all articles from the database.
    ///
    /// This should usually be used inside a transaction in combination with
    /// inserting new articles.
    async fn delete_all(connection: &mut SqliteConnection) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM articles")
            .execute(connection)
            .await
            .map(|_| ())
    }

    /// Insert an article into the database.
    async fn insert(&self, connection: &mut SqliteConnection) -> sqlx::Result<()> {
        let prices = serde_json::to_string(&self.prices)
            .map_err(Into::into)
            .map_err(sqlx::Error::Encode)?;

        sqlx::query(
            r#"
            INSERT INTO articles (id, designation, prices)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(&self.id)
        .bind(&self.designation)
        .bind(prices)
        .execute(connection)
        .await
        .map(|_| ())
    }

    /// Remove all articles from the database and insert a new set of articles.
    ///
    /// If any article fails to insert, a warning is logged, but the transaction
    /// is still committed. This ensures that we still insert as many articles as
    /// possible, even if some of them e.g. share the same barcode causing a
    /// unique constraint violation.
    pub async fn save_all(pool: SqlitePool, articles: Vec<Self>) -> sqlx::Result<()> {
        let mut transaction = pool.begin().await?;

        Self::delete_all(&mut transaction).await?;
        for article in articles {
            if let Err(error) = article.insert(&mut transaction).await {
                warn!("Failed to insert article: {error}");
            }
        }

        transaction.commit().await
    }

    /// Get the current price of the article.
    ///
    /// This may return `None` if the current date is not covered by
    /// any date range.
    pub fn current_price(&self) -> Option<Decimal> {
        self.price_for_date(&jiff::Zoned::now().date())
    }

    /// Get the price of the article for a specific date.
    ///
    /// This may return `None` if the date is not covered by any date range.
    pub fn price_for_date(&self, date: &jiff::civil::Date) -> Option<Decimal> {
        self.prices
            .iter()
            .find(|price| price.valid_from <= *date && price.valid_to >= *date)
            .map(|price| price.unit_price)
    }
}

/// A sale of an article to a member.
///
/// Sales are temporarily stored in the `sales` table before they are uploaded
/// to Vereinsflieger. This allows us to sell articles without an internet
/// connection and upload the sales later. This also works around the 500
/// request limit per day, since the remaining sales can be synchronized on
/// the next day.
#[derive(Debug, sqlx::FromRow)]
pub struct Sale {
    /// The unique ID of the sale.
    pub id: Text<Ulid>,
    /// The date of the sale.
    pub date: Text<jiff::civil::Date>,
    /// The member ID of the buyer (aka. "Mitgliedsnummer").
    pub member_id: String,
    /// The article ID of the sold article (aka. "Artikelnummer").
    pub article_id: String,
    /// The amount of articles sold.
    pub amount: u32,
}

impl Sale {
    /// Load all sales from the database.
    pub async fn load_all(pool: SqlitePool) -> sqlx::Result<Vec<Self>> {
        sqlx::query_as(
            r#"
            SELECT id, date, member_id, article_id, amount
            FROM sales
            "#,
        )
        .fetch_all(&pool)
        .await
    }

    /// Insert a sale into the database.
    async fn insert(&self, connection: &mut SqliteConnection) -> sqlx::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO sales (id, date, member_id, article_id, amount)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(self.id)
        .bind(self.date)
        .bind(&self.member_id)
        .bind(&self.article_id)
        .bind(self.amount)
        .execute(connection)
        .await
        .map(|_| ())
    }

    /// Insert multiple sales into the database.
    #[tracing::instrument(skip(pool))]
    pub async fn insert_all(pool: SqlitePool, sales: Vec<Sale>) -> sqlx::Result<()> {
        info!("Adding sales to database…");

        let mut transaction = pool.begin().await?;

        for sale in sales {
            sale.insert(&mut transaction).await?;
        }

        transaction.commit().await?;

        Ok(())
    }

    /// Delete a sale by its ID.
    pub async fn delete_by_id(pool: &SqlitePool, id: Ulid) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM sales WHERE id = $1")
            .bind(id.to_string())
            .execute(pool)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keycode_conversion() {
        let check = |input: &str, expected| {
            let key = vereinsflieger::Key {
                name: input.to_string(),
                title: "".to_string(),
            };

            assert_eq!(Member::parse_keycode(key).as_deref(), expected);
        };

        check("0005635570", Some("0005635570"));
        check("055FDF2", Some("0005635570"));
        check("S2017, A2711, 20€", None);
        check("20 Euro", None);
    }

    #[tokio::test]
    async fn test_duplicate_article_insertion() -> anyhow::Result<()> {
        let article1 = Article {
            id: "1".to_string(),
            designation: "Test Artikel 1".to_string(),
            prices: vec![],
        };

        let article2 = Article {
            id: "1".to_string(),
            designation: "Test Artikel 2".to_string(),
            prices: vec![],
        };

        let articles = vec![article1, article2];

        let pool = SqlitePool::connect(":memory:").await?;
        sqlx::migrate!().run(&pool).await?;

        Article::save_all(pool.clone(), articles).await?;

        let (count,): (u32,) = sqlx::query_as("SELECT COUNT(*) FROM articles")
            .fetch_one(&pool)
            .await?;

        assert_eq!(count, 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_duplicate_member_insertion() -> anyhow::Result<()> {
        let member1 = Member {
            keycode: "0005635570".to_string(),
            id: "1".to_string(),
            firstname: "John".to_string(),
            lastname: "Doe".to_string(),
            nickname: "".to_string(),
        };

        let member2 = Member {
            keycode: "0005635570".to_string(),
            id: "1".to_string(),
            firstname: "Jane".to_string(),
            lastname: "Doe".to_string(),
            nickname: "".to_string(),
        };

        let members = vec![member1, member2];

        let pool = SqlitePool::connect(":memory:").await?;
        sqlx::migrate!().run(&pool).await?;

        Member::save_all(pool.clone(), members).await?;

        let (count,): (u32,) = sqlx::query_as("SELECT COUNT(*) FROM members")
            .fetch_one(&pool)
            .await?;

        assert_eq!(count, 1);

        Ok(())
    }
}
