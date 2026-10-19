# The update check and crash reports (desktop editor only).

update-ask-title = Check for updates?
update-newer-title = A new version is out
update-title = Updates
update-ask-text = Audio Constructor can look once a day for a newer version and tell you when one is out. It only asks the project's release page; nothing about you or your work is sent.
update-check-daily = Check Daily
update-dont-check = Don't Check
update-change-later = Change it any time under Help.
# "Audio Constructor 0.15.0 is out. You have 0.14.0."
update-newer-text = Audio Constructor { $version } is out. You have { $current }.
update-download = Download
update-whats-new = What's New
update-skip = Skip This Version
update-later = Later
update-up-to-date = You have the newest version, { $version }.
update-failed = Couldn't check for updates: { $why }.
update-check-stopped = the check stopped unexpectedly

crash-title = Audio Constructor stopped unexpectedly
# $program is "audioc" or the editor's name from the report, or
# crash-the-editor.
crash-the-editor = the editor
crash-text =
    { $count ->
        [one] Last time, a bug stopped it. A report was saved from { $program }: the version, your system, and where it stopped. It's on this computer only; nothing has been sent.
       *[other] Last time, a bug stopped it. { $count } reports were saved, the newest from { $program }: the version, your system, and where it stopped. They're on this computer only; nothing has been sent.
    }
crash-report-text = Reporting it helps get the bug fixed: the project's issue form opens in your browser with the report filled in, for you to read, add what you were doing, and send, or not.
crash-report = Report…
crash-show-report = Show Report
crash-hide-report = Hide Report
crash-dismiss = Dismiss
crash-never = Don't offer crash reports again
crash-saved-as = Saved as { $path }
