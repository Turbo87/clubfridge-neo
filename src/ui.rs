use crate::running::{RunningClubFridge, Sale};
use crate::starting::StartingClubFridge;
use crate::state::{ClubFridge, Message, State};
use iced::border::rounded;
use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, row, scrollable, stack, text, Row};
use iced::Length::Fixed;
use iced::{color, Center, Element, Fill, Length, Right, Shrink, Theme};
use rust_decimal::Decimal;
use std::sync::Arc;

impl ClubFridge {
    pub fn theme(&self) -> Theme {
        Theme::Custom(Arc::new(iced::theme::Custom::new(
            "clubfridge".to_string(),
            iced::theme::Palette {
                background: color!(0x000000),
                text: color!(0xffffff),
                primary: color!(0x2E54C8),
                success: color!(0x4BD130),
                danger: color!(0xD5A30F),
            },
        )))
    }

    pub fn view(&self) -> Element<Message> {
        match &self.state {
            State::Starting(cf) => cf.view(),
            State::Setup(cf) => cf.view(),
            State::Running(cf) => cf.view(),
        }
    }
}

impl StartingClubFridge {
    pub fn view(&self) -> Element<Message> {
        let title = text("ClubFridge neo").size(36).width(Fill).align_x(Center);

        let status = if self.pool.is_none() {
            "Connecting to database…"
        } else if !self.migrations_finished {
            "Running database migrations…"
        } else {
            "Starting ClubFridge…"
        };

        let status = text(status)
            .color(color!(0xffee12))
            .size(24)
            .width(Fill)
            .align_x(Center);

        container(column![title, status].spacing(30))
            .height(Fill)
            .align_y(Center)
            .padding([20, 30])
            .into()
    }
}

impl RunningClubFridge {
    pub fn view(&self) -> Element<Message> {
        let title = self
            .user
            .as_ref()
            .map(|user| {
                if user.nickname.is_empty() {
                    text(format!(
                        "{} {} – Produkte scannen bitte",
                        user.firstname, user.lastname
                    ))
                } else {
                    text(format!("{} – Produkte scannen bitte", user.nickname))
                }
            })
            .unwrap_or(text("Bitte RFID Chip"));

        let update_available: Option<Element<Message>> = self.self_updated.as_ref().map(|_| {
            if self.update_button {
                let label = "Update verfügbar. Bitte Gerät neustarten!";
                text(label).size(24).into()
            } else {
                row![
                    text("Update verfügbar.").size(24),
                    button(
                        text("Jetzt updaten")
                            .color(color!(0xffffff))
                            .size(18)
                            .height(Fill)
                            .align_x(Center)
                            .align_y(Center)
                    )
                    .style(button::primary)
                    .padding([0, 10])
                    .on_press(Message::Shutdown),
                ]
                .spacing(10)
                .height(Shrink)
                .into()
            }
        });

        let sum = self.sales.iter().map(|item| item.total()).sum::<Decimal>();
        let sum = text(format!("Summe: {sum:.2}€"))
            .size(24)
            .width(Fill)
            .align_x(Right);

        let status_row = Row::with_capacity(2).push_maybe(update_available).push(sum);

        let mut cancel_label = "Abbruch".to_string();
        if let Some(timeout) = self.interaction_timeout {
            let secs_remaining = timeout.as_secs();
            if self.sales.is_empty() && secs_remaining < 15 {
                cancel_label.push_str(&format!(" ({}s)", secs_remaining));
            }
        }
        let cancel_button = button(
            text(cancel_label)
                .color(color!(0xffffff))
                .size(36)
                .align_x(Center),
        )
        .width(Fill)
        .style(button::danger)
        .padding([10, 20])
        .on_press_maybe(self.user.as_ref().map(|_| Message::Cancel));

        let mut pay_label = "Bezahlen".to_string();
        if let Some(timeout) = self.interaction_timeout {
            let secs_remaining = timeout.as_secs();
            if !self.sales.is_empty() && secs_remaining < 15 {
                pay_label.push_str(&format!(" ({}s)", secs_remaining));
            }
        }
        let pay_button = button(
            text(pay_label)
                .color(color!(0xffffff))
                .size(36)
                .align_x(Center),
        )
        .width(Fill)
        .style(button::success)
        .padding([10, 20])
        .on_press_maybe(self.user.as_ref().map(|_| Message::Pay));

        let content = column![
            title.size(36),
            scrollable(items(&self.sales))
                .height(Fill)
                .width(Fill)
                .anchor_bottom(),
            status_row,
            row![cancel_button, pay_button].spacing(10),
        ]
        .spacing(10);

        let mut stack = stack![content];

        if self.show_sale_confirmation {
            stack = stack.push(
                container(
                    container(
                        text("Danke für deinen Kauf")
                            .size(36)
                            .color(color!(0x000000)),
                    )
                    .style(|_theme: &Theme| {
                        container::background(color!(0xffffff)).border(rounded(10.))
                    })
                    .padding([15, 30]),
                )
                .width(Fill)
                .height(Fill)
                .align_x(Center)
                .align_y(Center),
            );
        }

        container(stack).padding([20, 30]).into()
    }
}

fn items(items: &[Sale]) -> Element<Message> {
    column(items.iter().map(sale_row)).spacing(10).into()
}

fn sale_row(sale: &Sale) -> Element<Message> {
    const AMOUNT_WIDTH: Length = Fixed(40.);
    const PRICE_WIDTH: Length = Fixed(80.);

    let amount = text(format!("{}x", sale.amount))
        .width(AMOUNT_WIDTH)
        .size(24)
        .align_x(Right)
        .wrapping(Wrapping::None);

    let article_name = text(&sale.article.designation).size(24).width(Fill);

    let unit_price = sale.article.current_price().unwrap_or_default();
    let unit_price = text(format!("{:.2}€", unit_price))
        .width(PRICE_WIDTH)
        .size(24)
        .color(color!(0x888888))
        .align_x(Right)
        .wrapping(Wrapping::None);

    let total_price = sale.total();
    let total_price = text(format!("{:.2}€", total_price))
        .width(PRICE_WIDTH)
        .size(24)
        .align_x(Right)
        .wrapping(Wrapping::None);

    row![amount, article_name, unit_price, total_price]
        .spacing(20)
        .into()
}
