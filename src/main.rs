mod database;
mod running;
mod starting;
mod state;
mod ui;

use crate::state::ClubFridge;

pub fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    ClubFridge::run()?;

    Ok(())
}
