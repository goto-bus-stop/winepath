use winepath::WineConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    ToUnix,
    ToWindows,
}

fn main() {
    let mut action = Action::ToUnix;
    let mut path = None;

    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "-u" => action = Action::ToUnix,
            "-w" => action = Action::ToWindows,
            _ => path = Some(arg),
        }
    } else {
        panic!("usage: winepath [OPTION] [PATH]")
    };

    let path = if let Some(path) = path {
        path
    } else if let Some(path) = std::env::args().nth(2) {
        path
    } else {
        panic!("usage: winepath [OPTION] [PATH]");
    };

    let config = WineConfig::from_env().unwrap();
    println!("{}", match action {
        Action::ToUnix => config.to_native_path(path).unwrap().to_string_lossy().to_string(),
        Action::ToWindows => {
            let path = std::fs::canonicalize(path).unwrap();
            config.to_wine_path(path).unwrap().to_string()
        }
    })
}
