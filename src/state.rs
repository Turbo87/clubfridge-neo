use crate::database;
use crate::log_viewer::{LogFileName, LogViewer, LogViewerGeneration};
use crate::popup::Popup;
use crate::running::RunningClubFridge;
use crate::setup::Setup;
use crate::starting::StartingClubFridge;
use iced::keyboard::{Key, Modifiers};
use iced::{application, window, Subscription, Task};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// The interval at which the app should check for updates of itself.
const SELF_UPDATE_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Default, clap::Parser)]
pub struct Options {
    /// Run in fullscreen
    #[arg(long)]
    fullscreen: bool,

    /// Run in fullscreen
    #[arg(long, default_value = "clubfridge.db?mode=rwc")]
    database: SqliteConnectOptions,

    /// Run in offline mode (no network requests)
    #[arg(long)]
    pub offline: bool,

    /// When an application update is available, show an "Update" button that
    /// quits the application. Should only be used when the application is
    /// automatically restarted by a supervisor.
    #[arg(long)]
    pub update_button: bool,
}

pub struct GlobalState {
    pub options: Options,

    /// The updated app version, if the app has been updated.
    pub self_updated: Option<String>,

    pub popup: Option<Popup>,

    /// The hidden log viewer, when it is open over the application screen.
    pub log_viewer: Option<LogViewer>,
}

impl GlobalState {
    fn self_update(&self) -> Task<Message> {
        let self_updated = self.self_updated.clone();
        Task::future(async move {
            let result = self_update(self_updated).await;
            let result = result.map_err(Arc::new);
            Message::SelfUpdateResult(result)
        })
    }

    /// Show a popup message to the user with the default timeout.
    pub fn show_popup(&mut self, message: impl Into<String>) -> Task<Message> {
        let message = message.into();

        debug!("Showing popup: {message}");
        let (popup, task) = Popup::new(message).with_timeout();

        self.popup = Some(popup);
        task
    }

    /// Hide the currently shown popup, if any.
    pub fn hide_popup(&mut self) {
        if self.popup.take().is_some() {
            debug!("Hiding popup");
        }
    }
}

pub struct ClubFridge {
    pub global_state: GlobalState,
    pub state: State,
    next_log_viewer_generation: u64,
}

/// The different states (or screens) the application can be in.
pub enum State {
    /// The application is starting up (connecting to the database, running
    /// database migrations, and checking for stored credentials).
    Starting(StartingClubFridge),

    /// The application is in the setup screen, where the user can enter their
    /// credentials. This state is only shown if no credentials are found in the
    /// database.
    Setup(Setup),

    /// The application is running and the user can interact with it.
    Running(RunningClubFridge),
}

impl ClubFridge {
    pub fn run() -> iced::Result {
        let options = <Options as clap::Parser>::parse();

        application(Self::new_from_clap, Self::update, Self::view)
            .theme(Self::theme)
            .subscription(Self::subscription)
            .resizable(true)
            .window(window::Settings {
                size: (800., 480.).into(),
                fullscreen: options.fullscreen,
                ..Default::default()
            })
            .run()
    }

    fn new_from_clap() -> (Self, Task<Message>) {
        let options = <Options as clap::Parser>::parse();
        Self::new(options)
    }

    pub fn new(options: Options) -> (Self, Task<Message>) {
        let connect_options = options.database.clone();
        let connect_task = Task::future(async move {
            info!("Connecting to database…");
            let pool_options = SqlitePoolOptions::default();
            match pool_options.connect_with(connect_options).await {
                Ok(pool) => Message::DatabaseConnected(pool),
                Err(err) => {
                    error!("Failed to connect to database: {err}");
                    Message::DatabaseConnectionFailed
                }
            }
        });

        let popup_message = format!("clubfridge-neo v{} gestartet", env!("CARGO_PKG_VERSION"));
        let (popup, popup_task) = Popup::new(popup_message).with_timeout();
        let popup = Some(popup);

        let startup_task = Task::batch([connect_task, popup_task, Task::done(Message::SelfUpdate)]);

        let global_state = GlobalState {
            options,
            self_updated: None,
            popup,
            log_viewer: None,
        };

        let cf = Self {
            global_state,
            state: State::Starting(StartingClubFridge::new()),
            next_log_viewer_generation: 1,
        };

        (cf, startup_task)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let subscription = match &self.state {
            State::Starting(cf) => cf.subscription(),
            State::Setup(cf) => cf.subscription(),
            State::Running(cf) => cf.subscription(),
        };

        Subscription::batch([
            subscription,
            iced::keyboard::listen().filter_map(|event| match event {
                iced::keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    repeat,
                    ..
                } if !is_repeated_control_shortcut(repeat, modifiers) => {
                    Some(Message::KeyPress(key, modifiers))
                }
                _ => None,
            }),
            iced::time::every(SELF_UPDATE_INTERVAL).map(|_| Message::SelfUpdate),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::KeyPress(key, modifiers) if is_log_viewer_shortcut(&key, modifiers) => {
                if self.global_state.log_viewer.take().is_some() {
                    return Task::none();
                }

                let generation = LogViewerGeneration::new(self.next_log_viewer_generation);
                self.next_log_viewer_generation += 1;
                let mut log_viewer = LogViewer::new(generation);
                let task = log_viewer.refresh_log_files();
                self.global_state.log_viewer = Some(log_viewer);
                return task;
            }

            Message::KeyPress(Key::Named(iced::keyboard::key::Named::Escape), _)
                if self.global_state.log_viewer.is_some() =>
            {
                self.global_state.log_viewer = None;
            }

            Message::KeyPress(_, _) if self.global_state.log_viewer.is_some() => {}

            Message::RefreshLogFiles => {
                if let Some(log_viewer) = &mut self.global_state.log_viewer {
                    return log_viewer.refresh_log_files();
                }
            }

            Message::CloseLogViewer => {
                self.global_state.log_viewer = None;
            }

            Message::LogFileListLoaded { generation, result } => {
                if let Some(log_viewer) = &mut self.global_state.log_viewer {
                    if log_viewer.generation() == generation {
                        return log_viewer.apply_log_file_list_result(result);
                    }
                }
            }

            Message::SelectLogFile(file_name) => {
                if let Some(log_viewer) = &mut self.global_state.log_viewer {
                    return log_viewer.load_selected_log_file(file_name);
                }
            }

            Message::LogFileContentsLoaded {
                generation,
                file_name,
                result,
            } => {
                if let Some(log_viewer) = &mut self.global_state.log_viewer {
                    if log_viewer.generation() == generation {
                        log_viewer.apply_log_file_contents(file_name, result);
                    }
                }
            }

            Message::GotoSetup(pool) => {
                self.state = State::Setup(Setup::new(pool));
            }

            Message::StartupComplete(pool, vereinsflieger) => {
                let (cf, task) = RunningClubFridge::new(pool, vereinsflieger);
                self.state = State::Running(cf);
                return task;
            }

            Message::SelfUpdate => {
                return self.global_state.self_update();
            }

            Message::SelfUpdateResult(result) => match result {
                Ok(self_update::Status::Updated(version)) => {
                    info!("App has been updated to version {version}");
                    self.global_state.self_updated = Some(version);
                }
                Ok(self_update::Status::UpToDate(_)) => {
                    info!("App is already up-to-date");
                }
                Err(err) => {
                    warn!("Failed to check for updates: {err}");
                }
            },

            Message::PopupTimeoutReached => {
                self.global_state.hide_popup();
            }

            Message::Shutdown => {
                info!("Shutting down…");
                return window::latest().and_then(window::close);
            }

            message => {
                return match &mut self.state {
                    State::Starting(cf) => cf.update(message, &mut self.global_state),
                    State::Setup(cf) => cf.update(message, &mut self.global_state),
                    State::Running(cf) => cf.update(message, &mut self.global_state),
                }
            }
        }

        Task::none()
    }
}

fn is_log_viewer_shortcut(key: &Key, modifiers: Modifiers) -> bool {
    modifiers.control()
        && !modifiers.alt()
        && !modifiers.logo()
        && matches!(key, Key::Character(character) if character.eq_ignore_ascii_case("l"))
}

fn is_repeated_control_shortcut(repeat: bool, modifiers: Modifiers) -> bool {
    repeat && modifiers.control()
}

async fn self_update(self_updated: Option<String>) -> anyhow::Result<self_update::Status> {
    let status = tokio::task::spawn_blocking(move || {
        info!("Checking for updates…");

        let current_version = self_updated.as_deref().unwrap_or(env!("CARGO_PKG_VERSION"));

        self_update::backends::github::Update::configure()
            .repo_owner("Turbo87")
            .repo_name("clubfridge-neo")
            .bin_name("clubfridge-neo")
            .current_version(current_version)
            .show_output(false)
            .no_confirm(true)
            .build()?
            .update()
    })
    .await??;

    Ok(status)
}

#[derive(Debug, Clone)]
pub enum Message {
    /// The database connection was successful.
    DatabaseConnected(SqlitePool),
    /// The database connection failed.
    DatabaseConnectionFailed,
    /// The database migrations were successful.
    DatabaseMigrated,
    /// The database migrations failed.
    DatabaseMigrationFailed,
    /// Credentials were found in the database.
    CredentialsFound(database::Credentials),
    /// The user should be taken to the setup screen to enter their credentials.
    GotoSetup(SqlitePool),
    /// The database lookup for credentials failed.
    CredentialLookupFailed,

    /// The user entered a club ID.
    SetClubId(String),
    /// The user entered an app key.
    SetAppKey(String),
    /// The user entered a username/email address.
    SetUsername(String),
    /// The user entered a password.
    SetPassword(String),
    /// The user submitted the setup form.
    SubmitSetup,
    /// Authentication with Vereinsflieger failed.
    AuthenticationFailed,

    /// Authentication with Vereinsflieger was successful, the application is
    /// transitioning to the running state.
    StartupComplete(SqlitePool, Option<vereinsflieger::Client>),

    /// The application should check for updates.
    SelfUpdate,
    /// The self-update check completed.
    SelfUpdateResult(Result<self_update::Status, Arc<anyhow::Error>>),
    /// The application should load the latest lists of members and articles
    /// from the Vereinsflieger API.
    LoadFromVF,
    /// The application should upload all sales to Vereinsflieger.
    UploadSalesToVF,
    /// The application received a key press event.
    KeyPress(Key, Modifiers),
    /// The user requested another scan of the application log directory.
    RefreshLogFiles,
    /// The user closed the hidden log viewer.
    CloseLogViewer,
    /// A scan of the application log directory completed.
    LogFileListLoaded {
        generation: LogViewerGeneration,
        result: Result<Vec<LogFileName>, Arc<std::io::Error>>,
    },
    /// The user selected a file in the hidden log viewer.
    SelectLogFile(LogFileName),
    /// A selected application log file finished loading.
    LogFileContentsLoaded {
        generation: LogViewerGeneration,
        file_name: LogFileName,
        result: Result<Vec<u8>, Arc<std::io::Error>>,
    },
    /// A "find member by keycode" query finished.
    FindMemberResult {
        input: String,
        result: Result<Option<database::Member>, Arc<sqlx::Error>>,
    },
    /// A "find article by barcode" query finished.
    FindArticleResult {
        input: String,
        result: Result<Option<database::Article>, Arc<sqlx::Error>>,
    },
    /// The user pressed the "Pay" button.
    Pay,
    /// The user pressed the "Cancel" button.
    Cancel,
    /// Decrement the automatic sale timeout until it reaches zero.
    DecrementTimeout,
    /// The popup timeout was reached, the popup should be closed.
    PopupTimeoutReached,
    /// Sales were successfully saved to the local database.
    SalesSaved,
    /// Saving sales to the local database failed.
    SavingSalesFailed,

    /// The application should shut down.
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::key::Named;

    #[tokio::test]
    async fn test_initial_state() {
        let (cf, _) = ClubFridge::new(Default::default());
        assert!(matches!(cf.state, State::Starting(_)));
    }

    #[tokio::test]
    async fn log_viewer_shortcuts_toggle_globally_and_consume_scanner_input() {
        let (mut cf, _) = ClubFridge::new(Default::default());
        let pool = SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let (running, _) = RunningClubFridge::new(pool, None);
        cf.state = State::Running(running);

        drop(cf.update(Message::KeyPress(
            Key::Character("l".into()),
            Modifiers::CTRL,
        )));
        assert!(cf.global_state.log_viewer.is_some());
        let first_generation = cf.global_state.log_viewer.as_ref().unwrap().generation();

        drop(cf.update(Message::KeyPress(
            Key::Character("1".into()),
            Modifiers::empty(),
        )));
        let State::Running(running) = &cf.state else {
            panic!("expected the running state");
        };
        assert!(running.input.is_empty());

        drop(cf.update(Message::KeyPress(
            Key::Named(Named::Escape),
            Modifiers::empty(),
        )));
        assert!(cf.global_state.log_viewer.is_none());

        drop(cf.update(Message::KeyPress(
            Key::Character("l".into()),
            Modifiers::CTRL,
        )));
        drop(cf.update(Message::CloseLogViewer));
        assert!(cf.global_state.log_viewer.is_none());

        drop(cf.update(Message::KeyPress(
            Key::Character("L".into()),
            Modifiers::CTRL | Modifiers::SHIFT,
        )));
        assert!(cf.global_state.log_viewer.is_some());
        let second_generation = cf.global_state.log_viewer.as_ref().unwrap().generation();
        assert_ne!(first_generation, second_generation);

        drop(cf.update(Message::LogFileListLoaded {
            generation: first_generation,
            result: Ok(Vec::new()),
        }));
        assert!(cf.global_state.log_viewer.as_ref().unwrap().is_busy());

        drop(cf.update(Message::LogFileListLoaded {
            generation: second_generation,
            result: Ok(Vec::new()),
        }));
        assert!(!cf.global_state.log_viewer.as_ref().unwrap().is_busy());

        drop(cf.update(Message::KeyPress(
            Key::Character("l".into()),
            Modifiers::CTRL,
        )));
        assert!(cf.global_state.log_viewer.is_none());
    }

    #[test]
    fn ignores_repeated_control_shortcuts() {
        assert!(is_repeated_control_shortcut(true, Modifiers::CTRL));
        assert!(is_repeated_control_shortcut(
            true,
            Modifiers::CTRL | Modifiers::SHIFT
        ));
        assert!(!is_repeated_control_shortcut(false, Modifiers::CTRL));
        assert!(!is_repeated_control_shortcut(true, Modifiers::empty()));
    }
}
