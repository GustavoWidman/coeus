//! the fleet as the dashboard's browser reads it. these shapes are its own:
//! the daemon's wire is positional postcard and job logs are JSON a newer
//! daemon must still read, so neither encoding may bend to suit a browser.
//! [derive] builds them from the domain, which stays the only source of truth.

pub mod derive;
pub mod transcript;
pub mod view;

#[cfg(test)]
mod test;
