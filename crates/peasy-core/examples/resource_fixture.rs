#[path = "../../../peas/tests/resource_fixture.rs"]
mod fixture;
fn main() {
    print!("{}", fixture::render());
}
