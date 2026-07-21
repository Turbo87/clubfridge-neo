# Hidden log viewer

## Interaction and layout

The log viewer is a hidden, full-screen application mode with no button or hint on the normal ClubFridge screens. Pressing `Ctrl+L` from startup, setup, or normal operation replaces the current screen with the viewer. Pressing `Ctrl+L` again, pressing `Escape`, or using the **Close** button returns to the previous screen. While the viewer is open, keyboard input is consumed by the viewer and never reaches credential fields or the RFID/barcode input.

Normal application activity continues behind it. Database startup, synchronization, update checks, popup timers, and an active purchase timeout are not paused. A purchase may therefore still complete or cancel automatically while someone reads the logs.

The viewer uses the full 800x480 area. A narrow pane on the left lists regular files matching `clubfridge-neo.*.log`, sorted alphabetically from A to Z. The selected filename is visually highlighted. On each opening, the alphabetically last file is selected, which corresponds to the newest file under the current date-based naming scheme.

The larger right pane displays the selected file in a vertically scrollable, monospace text view. Long lines wrap to the pane width. A header identifies the selected file and provides adjacent **Refresh** and **Close** buttons. Refreshing rescans the directory, re-sorts the file list, preserves the current selection when that file still exists, and reloads its contents. If the selected file disappeared, the newest remaining file becomes selected.

## File loading and failure behavior

Entering the viewer starts a fresh asynchronous scan of the relative `logs/` directory so file access does not block the UI. Only regular files whose names match `clubfridge-neo.*.log` are included. Directory traversal is not supported. Symlinks and subdirectories do not appear in the list.

Selecting a filename loads a complete snapshot of that file asynchronously. The result stays associated with the requested filename so rapidly selecting multiple files cannot allow an older read to replace the newer selection.

**Refresh** performs another directory scan and reloads the selected file. It retains that selection if possible. If the file was removed by log rotation, the newest remaining file is selected. If no files remain, the selection and content pane are cleared. **Close** immediately returns to the underlying application screen. An unfinished read may complete harmlessly but must not reopen or modify the closed viewer.

Expected empty and failure states are displayed inside the viewer instead of normal application popups:

- A missing or empty `logs/` directory shows `Keine Logdateien gefunden.`
- A directory scan failure shows a short German error message.
- A selected file that cannot be read shows the error in the content pane while leaving the file list usable.
- An empty log file shows an empty-state message instead of a blank unexplained pane.

The viewer is read-only. It does not delete, copy, download, search, or modify log files. The initial version reads the complete selected file without imposing a hidden size or line limit.

## Implementation plan

1. Add a focused `log_viewer` module that owns viewer state, directory scanning, file loading, selection reconciliation, and rendering. Reuse shared logging constants for the directory and filename pattern.
2. Add tests for filename filtering, alphabetical sorting, newest-file selection, selection preservation after refresh, removal fallback, empty directories, and read failures.
3. Move keyboard listening to the application level so `Ctrl+L` works from every screen without duplicate events. Test that `Ctrl+L` opens or closes the viewer, `Escape` closes it, and shortcut input never enters the scanner buffer.
4. Add asynchronous messages for scanning, selecting, loading, refreshing, and closing. Ignore load results for files that are no longer selected or a viewer that has closed.
5. Make the viewer take precedence in `ClubFridge::view()`, including over ordinary popups. Build the two-pane UI with a highlighted selected file, wrapped monospace content, German empty and error states, and adjacent **Refresh** and **Close** buttons.
6. Run focused tests through red-green-refactor, then the complete test suite, formatter, and Clippy. Review and commit the change without staging the pre-existing `Cargo.lock` modification.
