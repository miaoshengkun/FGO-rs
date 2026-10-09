fn main() {
    #[cfg(feature = "gui")]
    slint_build::compile("ui/app.slint").expect("compile desktop UI");
}
