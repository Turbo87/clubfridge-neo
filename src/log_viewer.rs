use crate::logging::{LOG_DIRECTORY, LOG_FILENAME_PREFIX, LOG_FILENAME_SUFFIX};
use crate::state::Message;
use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, rich_text, row, scrollable, span, text, Column};
use iced::{color, Center, Color, Element, Fill, Font, Length, Task};
use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

const ERROR_LOG_LINE_COLOR: Color = color!(0xFF5555);
const WARN_LOG_LINE_COLOR: Color = color!(0xD5A30F);
const DEBUG_LOG_LINE_COLOR: Color = color!(0x7AA2C8);
const TRACE_LOG_LINE_COLOR: Color = color!(0x888888);

/// A validated directory entry name for an application log file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogFileName(String);

impl LogFileName {
    fn as_str(&self) -> &str {
        &self.0
    }

    fn date_label(&self) -> &str {
        self.0
            .strip_prefix(LOG_FILENAME_PREFIX)
            .and_then(|name| name.strip_suffix(LOG_FILENAME_SUFFIX))
            .expect("log file names are validated during directory scans")
    }
}

/// Identifies one opening of the hidden log viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LogViewerGeneration(u64);

impl LogViewerGeneration {
    /// Creates a viewer generation from the app-owned opening counter.
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug)]
enum LogFileSelection {
    None,
    Loading(LogFileName),
    Loaded {
        file_name: LogFileName,
        contents: String,
    },
    Failed {
        file_name: LogFileName,
        error: Arc<io::Error>,
    },
}

impl LogFileSelection {
    fn file_name(&self) -> Option<&LogFileName> {
        match self {
            Self::None => None,
            Self::Loading(file_name)
            | Self::Loaded { file_name, .. }
            | Self::Failed { file_name, .. } => Some(file_name),
        }
    }
}

fn log_line_color(line: &str) -> Option<Color> {
    let mut fields = line.split_ascii_whitespace();
    let timestamp = fields.next()?;

    if timestamp.as_bytes().get(10) != Some(&b'T') {
        return None;
    }

    match fields.next() {
        Some("ERROR") => Some(ERROR_LOG_LINE_COLOR),
        Some("WARN") => Some(WARN_LOG_LINE_COLOR),
        Some("DEBUG") => Some(DEBUG_LOG_LINE_COLOR),
        Some("TRACE") => Some(TRACE_LOG_LINE_COLOR),
        _ => None,
    }
}

/// The file list, selection, and loaded snapshot shown in the log viewer.
#[derive(Debug)]
pub(crate) struct LogViewer {
    generation: LogViewerGeneration,
    log_files: Vec<LogFileName>,
    selection: LogFileSelection,
    scan_error: Option<Arc<io::Error>>,
    is_scanning: bool,
}

impl LogViewer {
    /// Creates an empty log viewer for one opening of the hidden mode.
    pub(crate) fn new(generation: LogViewerGeneration) -> Self {
        Self {
            generation,
            log_files: Vec::new(),
            selection: LogFileSelection::None,
            scan_error: None,
            is_scanning: false,
        }
    }

    /// Returns the generation that owns this viewer and its async results.
    pub(crate) fn generation(&self) -> LogViewerGeneration {
        self.generation
    }

    /// Reports whether a directory scan or file read is currently running.
    pub(crate) fn is_busy(&self) -> bool {
        self.is_scanning || matches!(self.selection, LogFileSelection::Loading(_))
    }

    /// Starts a fresh scan unless another viewer operation is still running.
    pub(crate) fn refresh_log_files(&mut self) -> Task<Message> {
        if self.is_busy() {
            return Task::none();
        }

        self.is_scanning = true;
        self.scan_error = None;
        scan_log_files_task(self.generation)
    }

    /// Applies a completed directory scan and loads the retained or newest selection.
    pub(crate) fn apply_log_file_list_result(
        &mut self,
        result: Result<Vec<LogFileName>, Arc<io::Error>>,
    ) -> Task<Message> {
        match result {
            Ok(log_files) => self
                .apply_log_file_list(log_files)
                .map_or_else(Task::none, |file_name| {
                    load_log_file_task(self.generation, file_name)
                }),
            Err(error) => {
                self.scan_error = Some(error);
                self.is_scanning = false;
                Task::none()
            }
        }
    }

    /// Selects and loads a log file that is present in the latest directory scan.
    pub(crate) fn load_selected_log_file(&mut self, file_name: LogFileName) -> Task<Message> {
        self.select_log_file(file_name)
            .map_or_else(Task::none, |file_name| {
                load_log_file_task(self.generation, file_name)
            })
    }

    /// Applies file bytes only when they belong to the current selection.
    pub(crate) fn apply_log_file_contents(
        &mut self,
        file_name: LogFileName,
        result: Result<Vec<u8>, Arc<io::Error>>,
    ) {
        if self.selected_log_file() != Some(&file_name) {
            return;
        }

        self.selection = match result {
            Ok(contents) => LogFileSelection::Loaded {
                file_name,
                contents: String::from_utf8_lossy(&contents).into_owned(),
            },
            Err(error) => LogFileSelection::Failed { file_name, error },
        };
    }

    /// Renders the full-screen, read-only application log viewer.
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let mut file_list = Column::new().spacing(5);

        if self.is_scanning {
            let status = if self.log_files.is_empty() {
                "Logdateien werden geladen…"
            } else {
                "Logdateien werden aktualisiert…"
            };
            file_list = file_list.push(text(status).size(14));
        }

        if let Some(error) = &self.scan_error {
            file_list = file_list.push(
                text(format!("Logdateien konnten nicht geladen werden: {error}"))
                    .size(14)
                    .color(color!(0xD5A30F)),
            );
        }

        if self.log_files.is_empty() && !self.is_scanning && self.scan_error.is_none() {
            file_list = file_list.push(text("Keine Logdateien gefunden.").size(14));
        }

        for file_name in &self.log_files {
            let is_selected = self.selected_log_file() == Some(file_name);
            let style = if is_selected {
                button::primary
            } else {
                button::secondary
            };
            let file_button = button(text(file_name.date_label()).size(14))
                .width(Fill)
                .padding([8, 10])
                .style(style);
            let file_button = if self.is_busy() {
                file_button
            } else {
                file_button.on_press(Message::SelectLogFile(file_name.clone()))
            };
            file_list = file_list.push(file_button);
        }

        let file_list = container(scrollable(file_list).height(Fill))
            .width(Length::Fixed(140.))
            .height(Fill)
            .padding(10)
            .style(container::bordered_box);

        let selected_file_name = self.selected_log_file().map_or("Logs", LogFileName::as_str);
        let refresh_button = button(text("Aktualisieren")).padding([8, 12]);
        let refresh_button = if self.is_busy() {
            refresh_button
        } else {
            refresh_button.on_press(Message::RefreshLogFiles)
        };
        let close_button = button(text("Schließen"))
            .padding([8, 12])
            .on_press(Message::CloseLogViewer);
        let header = row![
            text(selected_file_name)
                .size(18)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
            refresh_button,
            close_button,
        ]
        .spacing(10)
        .align_y(Center);

        let log_contents = rich_text(self.log_file_contents_spans())
            .font(Font::MONOSPACE)
            .size(14)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph);
        let log_contents = container(
            scrollable(log_contents)
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::new()
                        .width(32)
                        .scroller_width(32)
                        .spacing(4),
                ))
                .height(Fill),
        )
        .width(Fill)
        .height(Fill)
        .padding(10)
        .style(container::bordered_box);
        let contents_pane = column![header, log_contents].width(Fill).spacing(10);

        container(row![file_list, contents_pane].height(Fill).spacing(10))
            .width(Fill)
            .height(Fill)
            .padding([20, 30])
            .into()
    }

    fn selected_log_file(&self) -> Option<&LogFileName> {
        self.selection.file_name()
    }

    fn apply_log_file_list(&mut self, log_files: Vec<LogFileName>) -> Option<LogFileName> {
        let selected_file = self
            .selected_log_file()
            .filter(|selected| log_files.contains(selected))
            .cloned()
            .or_else(|| log_files.last().cloned());

        self.log_files = log_files;
        self.scan_error = None;
        self.is_scanning = false;
        self.selection = selected_file
            .as_ref()
            .map_or(LogFileSelection::None, |file_name| {
                LogFileSelection::Loading(file_name.clone())
            });

        selected_file
    }

    fn select_log_file(&mut self, file_name: LogFileName) -> Option<LogFileName> {
        if self.is_busy() || !self.log_files.contains(&file_name) {
            return None;
        }

        self.selection = LogFileSelection::Loading(file_name.clone());
        Some(file_name)
    }

    fn log_file_contents_text(&self) -> Cow<'_, str> {
        match &self.selection {
            LogFileSelection::None => {
                if let Some(error) = &self.scan_error {
                    Cow::Owned(format!("Logdateien konnten nicht geladen werden: {error}"))
                } else if self.is_scanning {
                    Cow::Borrowed("Logdateien werden geladen…")
                } else {
                    Cow::Borrowed("Keine Logdateien gefunden.")
                }
            }
            LogFileSelection::Loading(_) => Cow::Borrowed("Logdatei wird geladen…"),
            LogFileSelection::Loaded { contents, .. } if contents.is_empty() => {
                Cow::Borrowed("Logdatei ist leer.")
            }
            LogFileSelection::Loaded { contents, .. } => Cow::Borrowed(contents),
            LogFileSelection::Failed { error, .. } => {
                Cow::Owned(format!("Logdatei konnte nicht gelesen werden: {error}"))
            }
        }
    }

    fn log_file_contents_spans(&self) -> Vec<text::Span<'_, (), Font>> {
        match &self.selection {
            LogFileSelection::Loaded { contents, .. } if !contents.is_empty() => contents
                .split_inclusive('\n')
                .map(|line| span(line).color_maybe(log_line_color(line)))
                .collect(),
            _ => vec![span(self.log_file_contents_text())],
        }
    }
}

fn scan_log_files_task(generation: LogViewerGeneration) -> Task<Message> {
    Task::future(async move {
        let result = scan_log_files(PathBuf::from(LOG_DIRECTORY))
            .await
            .map_err(Arc::new);
        Message::LogFileListLoaded { generation, result }
    })
}

fn load_log_file_task(generation: LogViewerGeneration, file_name: LogFileName) -> Task<Message> {
    Task::future(async move {
        let result = tokio::fs::read(PathBuf::from(LOG_DIRECTORY).join(file_name.as_str()))
            .await
            .map_err(Arc::new);
        Message::LogFileContentsLoaded {
            generation,
            file_name,
            result,
        }
    })
}

async fn scan_log_files(directory: PathBuf) -> io::Result<Vec<LogFileName>> {
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut log_files = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_file() {
            continue;
        }

        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };

        if file_name.starts_with(LOG_FILENAME_PREFIX) && file_name.ends_with(LOG_FILENAME_SUFFIX) {
            log_files.push(LogFileName(file_name));
        }
    }

    log_files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(log_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TestLogDirectory {
        path: PathBuf,
    }

    impl TestLogDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "clubfridge-neo-log-viewer-test-{}",
                ulid::Ulid::new()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn create_file(&self, name: &str) {
            fs::write(self.path.join(name), "log contents").unwrap();
        }

        fn create_directory(&self, name: &str) {
            fs::create_dir(self.path.join(name)).unwrap();
        }
    }

    impl Drop for TestLogDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }

    #[tokio::test]
    async fn scans_matching_regular_log_files_in_alphabetical_order() {
        let directory = TestLogDirectory::new();
        directory.create_file("clubfridge-neo.2026-07-21.log");
        directory.create_file("clubfridge-neo.2026-07-19.log");
        directory.create_file("clubfridge-neo.2026-07-20.txt");
        directory.create_file("other.2026-07-20.log");
        directory.create_directory("clubfridge-neo.2026-07-18.log");

        let files = scan_log_files(directory.path.clone()).await.unwrap();

        assert_eq!(
            files,
            vec![
                LogFileName("clubfridge-neo.2026-07-19.log".to_string()),
                LogFileName("clubfridge-neo.2026-07-21.log".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn treats_a_missing_log_directory_as_an_empty_file_list() {
        let directory = std::env::temp_dir().join(format!(
            "clubfridge-neo-missing-log-viewer-test-{}",
            ulid::Ulid::new()
        ));

        let files = scan_log_files(directory).await.unwrap();

        assert!(files.is_empty());
    }

    #[test]
    fn derives_date_labels_from_log_file_names() {
        let file_name = LogFileName("clubfridge-neo.2026-07-21.log".to_string());

        assert_eq!(file_name.date_label(), "2026-07-21");
    }

    #[test]
    fn colors_log_lines_by_the_compact_formatter_level() {
        assert_eq!(
            log_line_color("2026-07-21T12:00:00.001Z ERROR clubfridge_neo: failed"),
            Some(ERROR_LOG_LINE_COLOR)
        );
        assert_eq!(
            log_line_color("2026-07-21T12:00:00.001Z  WARN clubfridge_neo: warning"),
            Some(WARN_LOG_LINE_COLOR)
        );
        assert_eq!(
            log_line_color("2026-07-21T12:00:00.001Z  INFO clubfridge_neo: started"),
            None
        );
        assert_eq!(
            log_line_color("2026-07-21T12:00:00.001Z DEBUG clubfridge_neo: update"),
            Some(DEBUG_LOG_LINE_COLOR)
        );
        assert_eq!(
            log_line_color("2026-07-21T12:00:00.001Z TRACE clubfridge_neo: detail"),
            Some(TRACE_LOG_LINE_COLOR)
        );
    }

    #[test]
    fn does_not_color_level_words_outside_the_compact_level_column() {
        assert_eq!(
            log_line_color("2026-07-21T12:00:00.001Z  INFO clubfridge_neo: ERROR was handled"),
            None
        );
        assert_eq!(log_line_color("continuation ERROR from nested error"), None);
        assert_eq!(log_line_color("continuation mentioning WARN"), None);
    }

    #[test]
    fn creates_one_colored_span_for_each_log_line() {
        let file_name = LogFileName("clubfridge-neo.2026-07-21.log".to_string());
        let mut viewer = LogViewer::new(LogViewerGeneration::new(1));
        viewer.apply_log_file_list(vec![file_name.clone()]);
        viewer.apply_log_file_contents(
            file_name,
            Ok(concat!(
                "2026-07-21T12:00:00.001Z ERROR clubfridge_neo: failed\n",
                "2026-07-21T12:00:01.001Z  INFO clubfridge_neo: recovered"
            )
            .as_bytes()
            .to_vec()),
        );

        let spans = viewer.log_file_contents_spans();

        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0].text,
            "2026-07-21T12:00:00.001Z ERROR clubfridge_neo: failed\n"
        );
        assert_eq!(spans[0].color, Some(ERROR_LOG_LINE_COLOR));
        assert_eq!(
            spans[1].text,
            "2026-07-21T12:00:01.001Z  INFO clubfridge_neo: recovered"
        );
        assert_eq!(spans[1].color, None);
    }

    #[test]
    fn refresh_preserves_the_selection_or_falls_back_to_the_newest_file() {
        let oldest = LogFileName("clubfridge-neo.2026-07-19.log".to_string());
        let selected = LogFileName("clubfridge-neo.2026-07-20.log".to_string());
        let newest = LogFileName("clubfridge-neo.2026-07-21.log".to_string());
        let mut viewer = LogViewer::new(LogViewerGeneration::new(1));

        assert_eq!(
            viewer.apply_log_file_list(vec![oldest.clone(), selected.clone()]),
            Some(selected.clone())
        );
        assert_eq!(viewer.selected_log_file(), Some(&selected));

        viewer.apply_log_file_contents(selected.clone(), Ok(Vec::new()));
        assert_eq!(viewer.select_log_file(oldest.clone()), Some(oldest.clone()));
        viewer.apply_log_file_contents(oldest.clone(), Ok(Vec::new()));
        assert_eq!(
            viewer.apply_log_file_list(vec![oldest.clone(), newest.clone()]),
            Some(oldest.clone())
        );
        assert_eq!(viewer.selected_log_file(), Some(&oldest));

        viewer.apply_log_file_contents(oldest, Ok(Vec::new()));
        assert_eq!(
            viewer.apply_log_file_list(vec![selected, newest.clone()]),
            Some(newest.clone())
        );
        assert_eq!(viewer.selected_log_file(), Some(&newest));

        viewer.apply_log_file_contents(newest, Ok(Vec::new()));
        assert_eq!(viewer.apply_log_file_list(Vec::new()), None);
        assert_eq!(viewer.selected_log_file(), None);
    }

    #[test]
    fn serializes_log_file_scans_and_reads() {
        let first = LogFileName("clubfridge-neo.2026-07-20.log".to_string());
        let second = LogFileName("clubfridge-neo.2026-07-21.log".to_string());
        let mut viewer = LogViewer::new(LogViewerGeneration::new(1));

        drop(viewer.refresh_log_files());
        assert!(viewer.is_busy());

        viewer.apply_log_file_list(vec![first.clone(), second.clone()]);
        assert!(viewer.is_busy());
        assert_eq!(viewer.select_log_file(first.clone()), None);

        viewer.apply_log_file_contents(second, Ok(b"latest contents".to_vec()));
        assert!(!viewer.is_busy());
        assert_eq!(viewer.select_log_file(first.clone()), Some(first));
    }

    #[test]
    fn ignores_file_contents_for_a_stale_selection() {
        let old_selection = LogFileName("clubfridge-neo.2026-07-20.log".to_string());
        let current_selection = LogFileName("clubfridge-neo.2026-07-21.log".to_string());
        let mut viewer = LogViewer::new(LogViewerGeneration::new(1));
        viewer.apply_log_file_list(vec![old_selection.clone(), current_selection.clone()]);

        viewer.apply_log_file_contents(old_selection, Ok(b"stale contents".to_vec()));
        assert_eq!(viewer.log_file_contents_text(), "Logdatei wird geladen…");

        viewer.apply_log_file_contents(current_selection, Ok(b"current \xffcontents".to_vec()));
        assert_eq!(viewer.log_file_contents_text(), "current \u{fffd}contents");
    }

    #[test]
    fn describes_empty_files_and_read_failures_in_german() {
        let file_name = LogFileName("clubfridge-neo.2026-07-21.log".to_string());
        let mut viewer = LogViewer::new(LogViewerGeneration::new(1));

        viewer.apply_log_file_list(Vec::new());
        assert_eq!(
            viewer.log_file_contents_text(),
            "Keine Logdateien gefunden."
        );

        viewer.apply_log_file_list(vec![file_name.clone()]);
        viewer.apply_log_file_contents(file_name.clone(), Ok(Vec::new()));
        assert_eq!(viewer.log_file_contents_text(), "Logdatei ist leer.");

        viewer.apply_log_file_contents(
            file_name,
            Err(Arc::new(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Zugriff verweigert",
            ))),
        );
        assert_eq!(
            viewer.log_file_contents_text(),
            "Logdatei konnte nicht gelesen werden: Zugriff verweigert"
        );
    }
}
