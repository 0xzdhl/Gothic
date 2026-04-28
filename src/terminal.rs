use std::io::{self, Write};

use anyhow::Result;
use crossterm::{
    cursor::MoveTo,
    execute,
    terminal::{Clear, ClearType},
};

pub fn clear_terminal() -> Result<()> {
    let mut stdout = io::stdout();

    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;

    stdout.flush()?;

    Ok(())
}
