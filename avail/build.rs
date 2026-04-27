use sesame_build::{Options, SesameBuilder};

fn main() {
    let mut builder = SesameBuilder::new(Options::new().verbose(true)).unwrap();
    builder.scrutinizer();
    builder.linter();
}
