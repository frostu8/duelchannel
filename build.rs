use std::env;

use git2::Repository;

fn main() {
    let repo = Repository::open(env::var("CARGO_MANIFEST_DIR").unwrap()).unwrap();

    let revspec = repo.revparse("HEAD").unwrap();

    let obj = revspec.from().unwrap();

    println!("cargo:rustc-env=GIT_COMMIT_HASH={}", obj.id());

    if let Ok(commit) = obj.clone().into_commit() {
        println!(
            "cargo:rustc-env=GIT_COMMIT_MESSAGE={}",
            commit.message().unwrap()
        );
    }
}
