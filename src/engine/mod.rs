//! the deployment engine: one task that owns every job's lifecycle on this
//! machine and publishes what the live views show

mod lane;
mod reconcile;

#[cfg(test)]
mod test;
