use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::widget::{button, column, container, row, scrollable, stack, text};
use iced::{application, color, Center, Element, Fill, Right, Subscription, Theme};
use std::sync::Arc;
use std::time::Duration;

pub fn main() -> iced::Result {
    application("ClubFridge neo", update, view)
        .theme(theme)
        .subscription(subscription)
        .resizable(true)
        .window_size((800., 480.))
        .run()
}

fn theme(_state: &State) -> Theme {
    Theme::Custom(Arc::new(iced::theme::Custom::new(
        "clubfridge".to_string(),
        iced::theme::Palette {
            background: color!(0x000000),
            text: color!(0xffffff),
            primary: color!(0xffffff),
            success: color!(0x4BD130),
            danger: color!(0xD5A30F),
        },
    )))
}

#[derive(Default)]
struct State {
    user: Option<String>,
    input: String,
    items: Vec<Item>,
    /// Show a confirmation screen until this timer runs out
    sale_confirmation_timer: u8,
}

#[derive(Debug, Clone)]
struct Item {
    amount: u16,
    description: String,
    price: f32,
}

impl Item {
    fn total(&self) -> f32 {
        self.amount as f32 * self.price
    }
}

#[derive(Debug, Clone)]
enum Message {
    KeyPress(Key),
    Pay,
    Cancel,
    DecreaseSaleConfirmationTimer,
}

fn subscription(state: &State) -> Subscription<Message> {
    let key_press_subscription =
        iced::keyboard::on_key_press(|key, _modifiers| Some(Message::KeyPress(key)));

    let mut subscriptions = vec![key_press_subscription];

    if state.sale_confirmation_timer != 0 {
        subscriptions.push(
            iced::time::every(Duration::from_secs(1))
                .map(|_| Message::DecreaseSaleConfirmationTimer),
        );
    }

    Subscription::batch(subscriptions)
}

fn update(state: &mut State, message: Message) {
    match message {
        Message::KeyPress(Key::Character(c)) => {
            state.input.push_str(c.as_str());
            state.sale_confirmation_timer = 0;
        }
        Message::KeyPress(Key::Named(Named::Enter)) => {
            if state.user.is_some() {
                state
                    .items
                    .iter_mut()
                    .find(|item| item.description == state.input)
                    .map(|item| {
                        item.amount += 1;
                    })
                    .unwrap_or_else(|| {
                        state.items.push(Item {
                            amount: 1,
                            description: state.input.clone(),
                            price: 0.5,
                        });
                    });
            } else {
                state.user = Some(state.input.clone());
            }

            state.input.clear();
            state.sale_confirmation_timer = 0;
        }
        Message::Pay => {
            state.user = None;
            state.items.clear();
            state.sale_confirmation_timer = 3;
        }
        Message::Cancel => {
            state.user = None;
            state.items.clear();
        }
        Message::DecreaseSaleConfirmationTimer => {
            state.sale_confirmation_timer = state.sale_confirmation_timer.saturating_sub(1);
        }
        _ => {}
    }
}

fn view(state: &State) -> Element<Message> {
    let sum = state.items.iter().map(|item| item.total()).sum::<f32>();

    let content = column![
        text(state.user.as_deref().unwrap_or("Bitte RFID Chip")).size(36),
        scrollable(items(&state.items))
            .height(Fill)
            .width(Fill)
            .anchor_bottom(),
        text(format!("Summe: € {sum:.2}"))
            .size(24)
            .align_x(Right)
            .width(Fill),
        row![
            button(
                text("Abbruch")
                    .color(color!(0xffffff))
                    .size(36)
                    .align_x(Center)
            )
            .width(Fill)
            .style(button::danger)
            .padding([10, 20])
            .on_press_maybe(state.user.as_ref().map(|_| Message::Cancel)),
            button(
                text("Bezahlen")
                    .color(color!(0xffffff))
                    .size(36)
                    .align_x(Center)
            )
            .width(Fill)
            .style(button::success)
            .padding([10, 20])
            .on_press_maybe(state.user.as_ref().map(|_| Message::Pay)),
        ]
        .spacing(10),
    ]
    .spacing(10);

    let mut stack = stack![content];

    if state.sale_confirmation_timer != 0 {
        stack = stack.push(
            container(
                container(
                    text("Danke für deinen Kauf")
                        .size(36)
                        .color(color!(0x000000)),
                )
                .style(|_theme: &Theme| container::background(color!(0xffffff)))
                .padding([15, 30]),
            )
            .width(Fill)
            .height(Fill)
            .align_x(Center)
            .align_y(Center),
        );
    }

    container(stack)
        .style(|_theme: &Theme| container::background(color!(0x000000)))
        .padding([20, 30])
        .into()
}

fn items(items: &[Item]) -> Element<Message> {
    row![
        column(
            items
                .iter()
                .map(|item| { text(format!("{}x", item.amount)).size(24).into() })
        )
        .align_x(Right)
        .spacing(10),
        column(
            items
                .iter()
                .map(|item| { text(&item.description).size(24).into() })
        )
        .width(Fill)
        .spacing(10),
        column(
            items
                .iter()
                .map(|item| { text(format!("{:.2}€", item.price,)).size(24).into() })
        )
        .align_x(Right)
        .spacing(10),
        column(
            items
                .iter()
                .map(|item| { text(format!("Gesamt {:.2}€", item.total())).size(24).into() })
        )
        .align_x(Right)
        .spacing(10),
    ]
    .spacing(20)
    .into()
}
