use std::env;
use std::io::{self, IsTerminal, Write};

lazy_static::lazy_static! {
    static ref USE_COLOR: bool = supports_color();
}

fn supports_color() -> bool {
    if env::var("NO_COLOR").is_ok() {
        return false;
    }
    io::stdout().is_terminal()
}

pub fn green(text: &str) -> String {
    if *USE_COLOR {
        format!("\x1b[32m{}\x1b[0m", text)
    } else {
        text.to_string()
    }
}

pub fn red(text: &str) -> String {
    if *USE_COLOR {
        format!("\x1b[31m{}\x1b[0m", text)
    } else {
        text.to_string()
    }
}

pub fn yellow(text: &str) -> String {
    if *USE_COLOR {
        format!("\x1b[33m{}\x1b[0m", text)
    } else {
        text.to_string()
    }
}

pub fn blue(text: &str) -> String {
    if *USE_COLOR {
        format!("\x1b[34m{}\x1b[0m", text)
    } else {
        text.to_string()
    }
}

pub fn bold(text: &str) -> String {
    if *USE_COLOR {
        format!("\x1b[1m{}\x1b[0m", text)
    } else {
        text.to_string()
    }
}

pub fn dim(text: &str) -> String {
    if *USE_COLOR {
        format!("\x1b[2m{}\x1b[0m", text)
    } else {
        text.to_string()
    }
}

pub fn print_error(msg: &str) {
    eprintln!("{}", red(&format!("Error: {}", msg)));
}

pub fn print_success(msg: &str) {
    println!("{}", green(msg));
}

pub fn print_warning(msg: &str) {
    println!("{}", yellow(msg));
}

pub fn print_info(msg: &str) {
    println!("{}", dim(msg));
}

pub fn prompt_yes_no(message: &str) -> bool {
    print!("{} [y/N]: ", message);
    io::stdout().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    input.trim().to_lowercase() == "y"
}
