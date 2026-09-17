pub mod app;
pub mod collector;
pub mod model;
mod terminal;
mod ui;
pub mod worker;

use std::{error::Error, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::{
    app::App,
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

                    app.handle_key(key.code);
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
