pub mod app;
pub mod collector;
pub mod model;
mod process_control;
mod terminal;
pub mod text;
pub mod theme;
mod ui;
pub mod worker;

use std::{error::Error, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::{
    app::App,
    process_control::terminate_process,
    terminal::TerminalSession,
    worker::{CollectorWorker, SnapshotStore},
};

fn main() -> Result<(), Box<dyn Error>> {
    run()
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut terminal = TerminalSession::start()?;
    let snapshot_store = SnapshotStore::default();
    let mut collector_worker =
        CollectorWorker::start(snapshot_store.clone(), Duration::from_secs(1));
    let mut app = App::new();

    while !app.should_quit() {
        let terminal_area = terminal.size()?;
        app.set_viewport_rows(ui::process_table_row_capacity(terminal_area, &app));

        if let Some(snapshot) = snapshot_store.latest()
            && app
                .snapshot()
                .is_none_or(|current| current.generation != snapshot.generation)
        {
            app.set_snapshot(snapshot);
        }

        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        break;
                    }

                    app.handle_key_with_modifiers(key.code, key.modifiers);
                    if let Some(target) = app.take_termination_request() {
                        match terminate_process(target.pid) {
                            Ok(()) => {}
                            Err(error) => app.set_status_message(format!(
                                "Could not terminate {}: {error}",
                                target.name
                            )),
                        }
                    }
                }
                // A resize event wakes the loop, and the next iteration redraws at
                // the new terminal dimensions.
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    collector_worker.stop();
    Ok(())
}
