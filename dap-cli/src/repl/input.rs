use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

pub const DEFAULT_PROMPT: &str = "dap› ";

pub struct ReplInput {
    editor: Option<DefaultEditor>,
}

impl ReplInput {
    pub fn new() -> Result<Self> {
        if io::stdin().is_terminal() {
            Ok(Self {
                editor: Some(DefaultEditor::new()?),
            })
        } else {
            Ok(Self { editor: None })
        }
    }

    pub fn uses_rustyline_prompt(&self) -> bool {
        self.editor.is_some()
    }

    pub fn read_line(&mut self) -> Result<Option<String>> {
        self.read_line_with_prompt(DEFAULT_PROMPT)
    }

    pub fn read_line_with_prompt(&mut self, prompt: &str) -> Result<Option<String>> {
        match &mut self.editor {
            Some(editor) => match editor.readline(prompt) {
                Ok(line) => {
                    if !line.trim().is_empty() {
                        editor.add_history_entry(line.as_str())?;
                    }
                    Ok(Some(line))
                }
                Err(ReadlineError::Interrupted) => Ok(None),
                Err(ReadlineError::Eof) => Ok(None),
                Err(err) => Err(err.into()),
            },
            None => {
                let stdin = io::stdin();
                let mut line = String::new();
                print!("{prompt}");
                io::stdout().flush().ok();
                match stdin.read_line(&mut line) {
                    Ok(0) => Ok(None),
                    Ok(_) => Ok(Some(line)),
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => Ok(None),
                    Err(err) => {
                        eprintln!("read error: {err}");
                        Ok(None)
                    }
                }
            }
        }
    }
}
