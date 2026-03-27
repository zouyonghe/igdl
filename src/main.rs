fn main() {
    if let Err(err) = igdl::run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
