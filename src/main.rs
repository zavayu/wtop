pub mod app;
pub mod collector;
pub mod model;
mod terminal;
mod ui;

use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::{app::App, terminal::TerminalSession};

fn main() -> Result<(), Box<dyn Error>> {
    run()
}

fn run() -> Result<(), Box<dyn Error>> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupt_flag = Arc::clone(&interrupted);

    ctrlc::set_handler(move || {
        interrupt_flag.store(true, Ordering::Relaxed);
    })?;

    let mut terminal = TerminalSession::start()?;
    let mut app = App::new();

    while !app.should_quit() && !interrupted.load(Ordering::Relaxed) {
        terminal.draw(ui::render)?;

        if event::poll(Duration::from_millis(250))? {
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

    Ok(())
}
