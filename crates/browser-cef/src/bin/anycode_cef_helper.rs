//! CEF subprocess helper (renderer / GPU / etc.).

fn main() {
    #[cfg(all(target_os = "macos", feature = "helper"))]
    {
        use cef::{args::Args, *};

        let args = Args::new();

        let _loader = {
            let loader = library_loader::LibraryLoader::new(
                &std::env::current_exe().expect("current_exe"),
                true,
            );
            assert!(loader.load(), "CEF helper failed to load framework");
            loader
        };

        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

        let code = execute_process(
            Some(args.as_main_args()),
            None::<&mut App>,
            std::ptr::null_mut(),
        );
        std::process::exit(code);
    }

    #[cfg(not(all(target_os = "macos", feature = "helper")))]
    {
        eprintln!("anycode-cef-helper is only built for macOS with the helper feature");
        std::process::exit(1);
    }
}
