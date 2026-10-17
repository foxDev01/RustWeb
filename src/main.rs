//! Binary entry point for the Task Tracker server.
//!
//! All routing lives in the library crate; this file only reads the
//! configuration, opens the database and starts the listener.

use std::error::Error;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;

use rust_task_tracker::{Database, app};
use tokio::net::TcpListener;

/// Port used when the `PORT` environment variable is absent.
const DEFAULT_PORT: u16 = 3000;

/// Database file used when `DATABASE_PATH` is absent.
///
/// It is resolved against the working directory, so the path is not a secret and
/// no credentials are stored in the source code.
const DEFAULT_DATABASE_PATH: &str = "tasks.db";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let database_path = read_configuration("DATABASE_PATH", DEFAULT_DATABASE_PATH)?;
    let db = Database::open(Path::new(&database_path))?;

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, read_port()?));
    let listener = TcpListener::bind(addr).await?;

    println!("Task Tracker running at http://{addr}");
    println!("Using database {database_path}");

    axum::serve(listener, app(db)).await?;
    Ok(())
}

/// Reads an optional configuration value.
///
/// "Variable not set" is a normal case and yields `default`, while a variable
/// that exists but cannot be decoded as Unicode is reported to the caller.
fn read_configuration(name: &str, default: &str) -> Result<String, Box<dyn Error>> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(default.to_owned()),
        // `as` makes the trait-object coercion explicit: otherwise this
        // expression would pin the error type to `Box<VarError>`.
        Err(err) => Err(Box::new(err) as Box<dyn Error>),
    }
}

/// Reads the listening port from the `PORT` environment variable.
///
/// An unset variable is not an error (the default applies), but a variable that
/// is set to something other than a port number is reported to the caller.
fn read_port() -> Result<u16, Box<dyn Error>> {
    match std::env::var("PORT") {
        Ok(raw) => Ok(raw.parse::<u16>()?),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_PORT),
        Err(err) => Err(Box::new(err) as Box<dyn Error>),
    }
}
