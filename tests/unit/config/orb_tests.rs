use super::*;

fn os(value: &str) -> Option<OsString> {
    Some(OsString::from(value))
}

#[test]
fn the_socket_path_comes_from_the_first_setting_that_is_usable() {
    let cwd = Path::new("/home/darren/jarvis");
    // (name, JARVIS_ORB_SOCKET, XDG_RUNTIME_DIR, expected)
    let cases = [
        (
            "an absolute setting is used as it is",
            os("/tmp/orb.sock"),
            os("/run/user/1000"),
            "/tmp/orb.sock",
        ),
        (
            "a relative setting is resolved against the working directory",
            os("run/orb.sock"),
            os("/run/user/1000"),
            "/home/darren/jarvis/run/orb.sock",
        ),
        (
            "no setting: the runtime directory",
            None,
            os("/run/user/1000"),
            "/run/user/1000/jarvis-orb.sock",
        ),
        (
            "an empty setting counts as unset",
            os(""),
            os("/run/user/1000"),
            "/run/user/1000/jarvis-orb.sock",
        ),
        (
            "no runtime directory: under the working directory",
            None,
            None,
            "/home/darren/jarvis/data/jarvis-orb.sock",
        ),
        (
            "an empty runtime directory counts as unset",
            None,
            os(""),
            "/home/darren/jarvis/data/jarvis-orb.sock",
        ),
        (
            "a relative runtime directory is not valid and is skipped",
            None,
            os("run/user"),
            "/home/darren/jarvis/data/jarvis-orb.sock",
        ),
    ];
    for (name, socket, runtime_dir, expected) in cases {
        let config = OrbConfig::from_values(socket, runtime_dir, cwd);
        assert_eq!(config.socket_path, Path::new(expected), "{name}");
    }
}
