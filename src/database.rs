use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use sqlx::types::Text;
use sqlx::{SqliteConnection, SqlitePool};
use tracing::{info, warn};
use ulid::Ulid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Credentials {
    pub club_id: u32,
    pub app_key: String,
    pub username: String,
    #[sqlx(try_from = "String")]
    pub password: SecretString,
}

impl Credentials {
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

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Member {
    pub keycode: String,
    pub id: String,
    pub firstname: String,
    pub lastname: String,
    #[allow(dead_code)]
    pub nickname: String,
}

impl Member {
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

    pub async fn delete_all(connection: &mut SqliteConnection) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM members")
            .execute(connection)
            .await
            .map(|_| ())
    }

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

    pub async fn save_all(pool: SqlitePool, members: Vec<Self>) -> sqlx::Result<()> {
        let mut transaction = pool.begin().await?;

        Self::delete_all(&mut transaction).await?;
        for member in members {
            if let Err(error) = member.insert(&mut transaction).await {
                warn!("Failed to insert member: {error}");
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
            Some(format!("{:0>10}", key))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Article {
    pub id: String,
    pub designation: String,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Price {
    pub valid_from: jiff::civil::Date,
    pub valid_to: jiff::civil::Date,
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
    pub async fn find_by_barcode(pool: SqlitePool, barcode: &str) -> sqlx::Result<Option<Self>> {
        sqlx::query_as(
            r#"
            SELECT id, designation, prices
            FROM articles
            WHERE lower(id) = lower($1)
            "#,
        )
        .bind(barcode)
        .fetch_optional(&pool)
        .await
    }

    pub async fn delete_all(connection: &mut SqliteConnection) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM articles")
            .execute(connection)
            .await
            .map(|_| ())
    }

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

    pub fn current_price(&self) -> Option<Decimal> {
        self.price_for_date(&jiff::Zoned::now().date())
    }

    pub fn price_for_date(&self, date: &jiff::civil::Date) -> Option<Decimal> {
        self.prices
            .iter()
            .find(|price| price.valid_from <= *date && price.valid_to >= *date)
            .map(|price| price.unit_price)
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct Sale {
    pub id: Text<Ulid>,
    pub date: Text<jiff::civil::Date>,
    pub member_id: String,
    pub article_id: String,
    pub amount: u32,
}

impl Sale {
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
