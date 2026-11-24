# Privacy

Blind Passcode Generator collects nothing about you.

- There is no account, no analytics and no advertising.
- The app's own code makes no connections to the internet, and nothing is
  sent to its author or to anyone else. It works with the internet off.
- On Windows, the Microsoft part that draws the app's window does contact
  Microsoft: see [WebView2 on Windows](#webview2-on-windows).
- Everything the app saves stays in files on your own computer.
- **Donate**, in Settings, opens a page on ko-fi.com in your web browser. The
  app sends nothing there itself, and nothing in the app changes if you give.
- **Send feedback**, in Settings, opens a new message in your mail app.
  Nothing goes anywhere unless you send it.

## What the app saves, and where

- **Saved codes.** Each one is a small file in the `.locker` folder in your
  home folder. It holds the name you typed, the date, and the code sealed
  behind a time-lock. Text you lock away yourself is kept the same way.
- **Backups, if you choose one.** At the end of a setup the app offers to put
  a copy of that file in a synced folder you already have, such as OneDrive,
  iCloud Drive or Dropbox. The copy is time-locked too. Where it goes from
  there is up to that service.
- **Recently deleted.** A deleted code waits 30 days in a `deleted` folder
  beside the others, then it is removed. If you had revealed the code before
  deleting it, the app also keeps it there in plain text for those 30 days,
  on this computer only.

Uninstalling the app leaves these files where they are. Delete the `.locker`
folder yourself to remove them.

## WebView2 on Windows

The app shows its screens with WebView2, a part of Windows made by Microsoft.
WebView2 sends its own diagnostic data and crash reports to Microsoft, as it
does in every app that uses it. The app doesn't need this: it works with the
internet off. You can limit what is sent in Windows Settings, under Privacy &
security, Diagnostics & feedback. Microsoft describes it in
[Data and privacy in WebView2](https://learn.microsoft.com/microsoft-edge/webview2/concepts/data-privacy).

If a PC doesn't have WebView2, the installer downloads it from Microsoft.

## Questions

Write to <blindpasscodegen@gmail.com>, or open an issue at
<https://github.com/uncleyam/blind-passcode-generator/issues>.
