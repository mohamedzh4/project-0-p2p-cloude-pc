// Keeps a console window from appearing behind the UI on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `--print-hardware` reports what the agent detects without opening the
    // window, which is what a host will be asked for when a match looks wrong.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--print-hardware")
    {
        match serde_json::to_string_pretty(&host_agent_lib::detect_hardware()) {
            Ok(report) => println!("{report}"),
            Err(error) => eprintln!("could not report hardware: {error}"),
        }

        return;
    }

    host_agent_lib::run()
}
