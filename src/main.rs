mod database;
mod log_viewer;
mod logging;
mod popup;
mod running;
mod setup;
mod starting;
mod state;
mod ui;

use crate::state::ClubFridge;

pub fn main() -> anyhow::Result<()> {
    logging::init()?;

    ClubFridge::run()?;

    Ok(())
}
