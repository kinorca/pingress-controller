mod controller;

pub use controller::*;

#[macro_export]
macro_rules! try_with_log {
    ($trying:expr) => {
        $trying.inspect_err(|e| ::log::error!("Error: {e}"))?
    };
}
