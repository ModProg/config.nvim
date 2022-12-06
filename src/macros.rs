macro_rules! continue_on_error {
    ($expr:expr, $err:ident, $($format:tt)*) => {
        match $expr {
            Ok(value) => value,
            Err($err) => {
                api::notify(&format!($($format)*), LogLevel::Error, &NotifyOpts::default()).unwrap();
                continue;
            }
        }
    };
    ($expr:expr, $($format:tt)*) => {
        if let Ok(value) = $expr {
             value
        } else {
            api::notify(&format!($($format)*), LogLevel::Error, None).unwrap();
            continue;
        }
    };
}
macro_rules! do_on_error {
    ($expr:expr, $do:expr, $err:ident, $($format:tt)*) => {
        match $expr {
            Ok(value) => value,
            Err($err) => {
                api::notify(&format!($($format)*), LogLevel::Error, &NotifyOpts::default()).unwrap();
                $do;
            }
        }
    };
    ($expr:expr, $do:expr, $($format:tt)*) => {
        if let Ok(value) = $expr {
             value
        } else {
            api::notify(&format!($($format)*), LogLevel::Error, None).unwrap();
            $do;
        }
    };
}
macro_rules! log_error {
    ($($format:tt)*) => {
        nvim_oxi::api::notify(&format!($($format)*), nvim_oxi::api::types::LogLevel::Error, &nvim_oxi::api::opts::NotifyOpts::default()).unwrap();
    };
}
