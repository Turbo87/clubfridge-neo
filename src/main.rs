mod database;
mod logging;
mod popup;
mod running;
mod setup;
mod starting;
mod state;
mod ui;
mod vereinsflieger;

use crate::state::{ClubFridge, Options};

pub fn main() -> anyhow::Result<()> {
    logging::init()?;

    let options = <Options as clap::Parser>::parse();

    ClubFridge::run(options)?;

    Ok(())
}
