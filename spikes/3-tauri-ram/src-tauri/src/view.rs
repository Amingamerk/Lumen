use serde::Serialize;

/// The screen the app opens on, so each RAM scenario can be launched without clicking.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    Home,
    List,
}

impl View {
    pub fn from_args<I>(args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        if args.into_iter().any(|arg| arg.as_ref() == "--list") {
            View::List
        } else {
            View::Home
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_starts_on_home_without_a_flag() {
        assert_eq!(View::from_args(["spike.exe"]), View::Home);
        assert_eq!(View::from_args(["spike.exe", "--listing"]), View::Home);
    }

    #[test]
    fn list_flag_starts_on_the_track_list() {
        assert_eq!(View::from_args(["spike.exe", "--list"]), View::List);
    }
}
