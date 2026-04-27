use sesame_build::{Options, SesameBuilder};

fn main() {
    let mut builder = SesameBuilder::new(Options::new().verbose(false)).unwrap();
    builder.scrutinizer();
    builder.linter();
}
