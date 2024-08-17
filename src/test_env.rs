use std::env;

fn main() {
    println!("AWS_ACCESS_KEY_ID: {:?}", env::var("AWS_ACCESS_KEY_ID"));
    println!(
        "AWS_SECRET_ACCESS_KEY: {:?}",
        env::var("AWS_SECRET_ACCESS_KEY")
    );
}
