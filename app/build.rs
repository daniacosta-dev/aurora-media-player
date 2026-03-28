fn main() {
    println!("cargo:rustc-link-arg=-lplacebo");
    glib_build_tools::compile_resources(
        &["../data/resources"],
        "../data/resources/resources.gresource.xml",
        "aurora-media.gresource",
    );
}
