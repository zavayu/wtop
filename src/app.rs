use crossterm::event::KeyCode;

/// Owns the application state that is independent of terminal rendering.
///
/// Process snapshots, sorting, filtering, and viewport state will be added in
/// later milestones. Keeping this state separate from `ui` makes those changes
/// testable without a terminal.
#[derive(Debug, Default)]
pub struct App {
    should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn handle_key(&mut self, key: KeyCode) {
        if matches!(key, KeyCode::Char('q') | KeyCode::Esc) {
            self.should_quit = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::App;
    use crossterm::event::KeyCode;

    #[test]
    fn quit_keys_request_a_clean_exit() {
        for key in [KeyCode::Char('q'), KeyCode::Esc] {
            let mut app = App::new();
            app.handle_key(key);
            assert!(app.should_quit());
        }
    }
}
