//! Shows the progress of the image pull while fbkd creates a sandbox: a progress bar on a
//! terminal, and otherwise a line when the download starts and one when it ends. Shows nothing
//! when no progress arrives, for example because the image is cached.

use std::io::{self, IsTerminal, Stderr, Write};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::api::ImagePullProgress;

/// Template of the bar when the size of the image is known.
const BAR_TEMPLATE: &str =
    "Pulling {msg} [{bar:30}] {binary_bytes} / {binary_total_bytes} ({percent}%)";

/// Template of the bar when the manifest has no layer sizes.
const BYTES_TEMPLATE: &str = "Pulling {msg} {binary_bytes}";

/// Where the progress of a pull goes.
enum Output<W> {
    /// A progress bar, updated in place. It stays hidden until the pull starts and the target
    /// is taken, so it's never drawn without its style.
    Bar {
        bar: ProgressBar,
        target: Option<ProgressDrawTarget>,
    },
    /// A line when the pull starts and one when it ends.
    Lines(W),
}

/// Shows the progress of an image pull from the progress messages of a start stream. An
/// unfinished bar is cleared when the view is dropped, so an error shows on a clean line.
pub struct PullProgressView<W: Write = Stderr> {
    output: Output<W>,
    image: Option<String>,
    finished: bool,
}

impl PullProgressView {
    /// Returns a view that shows a bar when stderr is a terminal, and lines otherwise.
    pub fn stderr() -> Self {
        if io::stderr().is_terminal() {
            Self::new(Output::bar(ProgressDrawTarget::stderr()))
        } else {
            Self::new(Output::Lines(io::stderr()))
        }
    }
}

impl<W> Output<W> {
    /// Returns a bar that draws on the target once the pull starts.
    fn bar(target: ProgressDrawTarget) -> Self {
        Self::Bar {
            bar: ProgressBar::hidden(),
            target: Some(target),
        }
    }
}

impl<W: Write> PullProgressView<W> {
    fn new(output: Output<W>) -> Self {
        Self {
            output,
            image: None,
            finished: false,
        }
    }

    /// Shows a progress message, and finishes the pull when it is the last one.
    pub fn update(&mut self, progress: &ImagePullProgress) {
        if self.finished {
            return;
        }

        if self.image.is_none() {
            self.begin(progress);
        }

        if let Output::Bar { bar, .. } = &self.output {
            bar.set_position(progress.downloaded_bytes);
        }

        if progress.complete {
            self.finish();
        }
    }

    /// Finishes a pull that is being shown. Does nothing when no pull was shown.
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }

        let Some(image) = &self.image else {
            return;
        };

        match &mut self.output {
            Output::Bar { bar, .. } => bar.finish(),
            // A failed write to stderr leaves nowhere to report it.
            Output::Lines(writer) => drop(writeln!(writer, "Pulled image {image}")),
        }

        self.finished = true;
    }

    /// Starts showing the pull of the image in the message.
    fn begin(&mut self, progress: &ImagePullProgress) {
        let image = progress.image.clone();

        match &mut self.output {
            Output::Bar { bar, target } => show_bar(bar, target.take(), progress),
            Output::Lines(writer) => drop(writeln!(writer, "Pulling image {image}...")),
        }

        self.image = Some(image);
    }
}

/// Gives the bar the template that fits the progress, with the image as its message, then
/// shows it on the target.
fn show_bar(bar: &ProgressBar, target: Option<ProgressDrawTarget>, progress: &ImagePullProgress) {
    let template = match progress.total_bytes {
        Some(total) => {
            bar.set_length(total);
            BAR_TEMPLATE
        }
        None => BYTES_TEMPLATE,
    };

    // The templates are constants with valid keys, so parsing them can't fail.
    if let Ok(style) = ProgressStyle::with_template(template) {
        bar.set_style(style.progress_chars("=> "));
    }

    bar.set_message(progress.image.clone());

    if let Some(target) = target {
        bar.set_draw_target(target);
    }
}

impl<W: Write> Drop for PullProgressView<W> {
    /// Clears a bar that didn't finish, for example because the pull failed.
    fn drop(&mut self) {
        if let (Output::Bar { bar, .. }, Some(_), false) =
            (&self.output, &self.image, self.finished)
        {
            bar.finish_and_clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indicatif::{InMemoryTerm, TermLike};
    use std::sync::{Arc, Mutex};

    /// A terminal that records every string written to it.
    #[derive(Debug, Clone, Default)]
    struct RecordingTerm(Arc<Mutex<Vec<String>>>);

    impl TermLike for RecordingTerm {
        fn width(&self) -> u16 {
            120
        }

        fn move_cursor_up(&self, _: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_down(&self, _: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_right(&self, _: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_left(&self, _: usize) -> io::Result<()> {
            Ok(())
        }

        fn write_line(&self, s: &str) -> io::Result<()> {
            self.write_str(s)
        }

        fn write_str(&self, s: &str) -> io::Result<()> {
            self.0.lock().unwrap().push(s.to_string());
            Ok(())
        }

        fn clear_line(&self) -> io::Result<()> {
            Ok(())
        }

        fn flush(&self) -> io::Result<()> {
            Ok(())
        }
    }

    const IMAGE: &str = "ghcr.io/wmeints/firebrick-base:test";
    const MIB: u64 = 1024 * 1024;

    fn progress(downloaded_mib: u64, total_mib: Option<u64>) -> ImagePullProgress {
        ImagePullProgress {
            image: IMAGE.to_string(),
            downloaded_bytes: downloaded_mib * MIB,
            total_bytes: total_mib.map(|total| total * MIB),
            complete: false,
        }
    }

    fn completed(downloaded_mib: u64, total_mib: Option<u64>) -> ImagePullProgress {
        ImagePullProgress {
            complete: true,
            ..progress(downloaded_mib, total_mib)
        }
    }

    fn terminal_view() -> (PullProgressView<Vec<u8>>, InMemoryTerm) {
        let term = InMemoryTerm::new(4, 120);
        let target = ProgressDrawTarget::term_like(Box::new(term.clone()));
        let view = PullProgressView::new(Output::bar(target));

        (view, term)
    }

    fn lines(messages: &[ImagePullProgress], finish: bool) -> String {
        let mut view = PullProgressView::new(Output::Lines(Vec::new()));

        for message in messages {
            view.update(message);
        }

        if finish {
            view.finish();
        }

        let Output::Lines(written) = &view.output else {
            unreachable!("the view writes lines");
        };

        String::from_utf8(written.clone()).unwrap()
    }

    #[test]
    fn bar_shows_downloaded_and_total_bytes_with_a_percentage() {
        let (mut view, term) = terminal_view();

        view.update(&progress(100, Some(400)));

        let contents = term.contents();
        assert!(
            contents.starts_with(&format!("Pulling {IMAGE} [")),
            "{contents}"
        );
        assert!(
            contents.ends_with("100.00 MiB / 400.00 MiB (25%)"),
            "{contents}"
        );
    }

    #[test]
    fn bar_is_only_drawn_with_its_style() {
        let term = RecordingTerm::default();
        let target = ProgressDrawTarget::term_like_with_hz(Box::new(term.clone()), 255);
        let mut view = PullProgressView::<Vec<u8>>::new(Output::bar(target));

        view.update(&progress(100, Some(400)));
        view.update(&completed(400, Some(400)));

        let frames = term.0.lock().unwrap().clone();
        let drawn: Vec<_> = frames.iter().filter(|s| !s.trim().is_empty()).collect();
        assert!(!drawn.is_empty());
        assert!(
            drawn
                .iter()
                .all(|frame| frame.starts_with(&format!("Pulling {IMAGE} ["))),
            "{drawn:?}"
        );
    }

    #[test]
    fn bar_shows_downloaded_bytes_only_without_a_total() {
        let (mut view, term) = terminal_view();

        view.update(&progress(100, None));

        assert_eq!(term.contents(), format!("Pulling {IMAGE} 100.00 MiB"));
    }

    #[test]
    fn bar_stays_finished_when_the_pull_completes() {
        let (mut view, term) = terminal_view();

        view.update(&progress(100, Some(400)));
        view.update(&completed(400, Some(400)));
        drop(view);

        let contents = term.contents();
        assert!(
            contents.ends_with("400.00 MiB / 400.00 MiB (100%)"),
            "{contents}"
        );
    }

    #[test]
    fn bar_ends_at_the_total_when_the_sandbox_starts_without_a_last_message() {
        let (mut view, term) = terminal_view();

        view.update(&progress(100, Some(400)));
        view.finish();
        drop(view);

        let contents = term.contents();
        assert!(
            contents.ends_with("400.00 MiB / 400.00 MiB (100%)"),
            "{contents}"
        );
    }

    #[test]
    fn bar_is_cleared_when_the_pull_fails() {
        let (mut view, term) = terminal_view();

        view.update(&progress(100, Some(400)));
        drop(view);

        assert_eq!(term.contents(), "");
    }

    #[test]
    fn bar_shows_nothing_without_progress() {
        let (mut view, term) = terminal_view();

        view.finish();
        drop(view);

        assert_eq!(term.contents(), "");
    }

    #[test]
    fn lines_show_the_start_and_end_of_the_pull_only() {
        let messages = [
            progress(100, Some(400)),
            progress(200, Some(400)),
            completed(400, Some(400)),
            // Messages after the last one are ignored.
            progress(400, Some(400)),
        ];

        assert_eq!(
            lines(&messages, true),
            format!("Pulling image {IMAGE}...\nPulled image {IMAGE}\n")
        );
    }

    #[test]
    fn lines_end_the_pull_when_the_sandbox_starts_before_it_completed() {
        assert_eq!(
            lines(&[progress(100, None)], true),
            format!("Pulling image {IMAGE}...\nPulled image {IMAGE}\n")
        );
    }

    #[test]
    fn lines_show_nothing_without_progress() {
        assert_eq!(lines(&[], true), "");
    }
}
