//! The one way a command writes to stdout. A write that fails ends the process here: a closed
//! pipe (`| head`) is not an error and exits 0 quietly; anything else (a full disk, `/dev/full`)
//! prints one `error:` line and exits 3. Rust ignores SIGPIPE, so without this `println!` panics.

use std::fmt::Arguments;
use std::io::Write;

/// Exit code for an unwritable stdout; the usage and setup code of the exit-code table.
const EXIT_WRITE: i32 = 3;

/// Write formatted text and flush it, so a prompt shows and an error surfaces at this call.
pub fn write(args: Arguments<'_>) {
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout.write_fmt(args).and_then(|()| stdout.flush()) {
        give_up(&e);
    }
}

/// Write raw bytes and flush, with the same failure handling as [`write`].
pub fn write_bytes(bytes: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        give_up(&e);
    }
}

fn give_up(e: &std::io::Error) -> ! {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    eprintln!("error: cannot write output: {e}");
    std::process::exit(EXIT_WRITE);
}

/// `print!` for stdout that exits instead of panicking when the write fails.
macro_rules! out {
    ($($arg:tt)*) => { $crate::out::write(format_args!($($arg)*)) };
}

/// `println!` counterpart of [`out!`].
macro_rules! outln {
    () => { $crate::out::write(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::out::write(format_args!("{}\n", format_args!($($arg)*))) };
}
