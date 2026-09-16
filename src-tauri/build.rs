fn main() {
    // Settings used to be generated here from a JSON file next to the source, which is why a
    // downloaded build only ever worked on the machine that built it. They are now read at run
    // time from the user's own configuration directory, written by the setup wizard, so there is
    // nothing about one person's computer left to compile in.
    tauri_build::build();
}
