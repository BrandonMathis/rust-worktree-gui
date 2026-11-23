use iced::{Task, window};
use rfd::{AsyncMessageDialog, MessageButtons, MessageDialogResult};

pub fn show(
    id: window::Id,
    title: &'static str,
    description: String,
    confirm: &'static str,
) -> Task<bool> {
    // Construct the alert on the UI thread and attach it to the app window.
    window::run(id, move |parent| {
        AsyncMessageDialog::new()
            .set_parent(parent)
            .set_title(title)
            .set_description(description)
            .set_buttons(MessageButtons::OkCancelCustom(
                confirm.into(),
                "Cancel".into(),
            ))
            .show()
    })
    .then(move |answer| Task::perform(answer, move |answer| is_confirmed(answer, confirm)))
}

fn is_confirmed(answer: MessageDialogResult, confirm: &str) -> bool {
    matches!(answer, MessageDialogResult::Custom(button) if button == confirm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_named_confirmation_button_accepts() {
        for confirm in ["Quit", "Remove Project"] {
            assert!(is_confirmed(
                MessageDialogResult::Custom(confirm.into()),
                confirm
            ));
            assert!(!is_confirmed(
                MessageDialogResult::Custom("Cancel".into()),
                confirm
            ));
            assert!(!is_confirmed(MessageDialogResult::Cancel, confirm));
        }
    }
}
