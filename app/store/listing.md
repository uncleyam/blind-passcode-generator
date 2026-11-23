# Microsoft Store listing

What to paste into Partner Center for a submission, page by page. Anything
not named here can stay as Partner Center has it.

## Pricing and availability

- Markets: all
- Visibility: Public audience
- Price: Free

## Properties

- Category: Utilities & tools
- Does this product access, collect or transmit personal information? No
- Privacy policy URL: <https://github.com/uncleyam/blind-passcode-generator/blob/main/PRIVACY.md>
- Website: <https://github.com/uncleyam/blind-passcode-generator>
- Support contact info: `blindpasscodegen@gmail.com`
- Product declarations: tick "This app allows users to make purchases, but
  does not use the Microsoft Store commerce system", or the box nearest to
  that wording. It is for Donate, which opens Ko-fi in the browser. Store
  policy 10.8.2 asks for payments outside the Store to be noted here.

## Age ratings

Choose the type for utilities and other apps, then answer No to every
question. The app has no violence, no chat, no purchases and no sharing of
any kind. Donate is not a purchase: it sells nothing and opens in the
browser.

## Packages

Upload `Blind-Passcode-Generator-store-x64.msix`. It is inside the
`microsoft-store-package` download on the Release run's page, under
Artifacts. Partner Center checks it as soon as it is uploaded.

## Store listing (English)

### Description

```
Set a passcode that you yourself don't know.

A Screen Time limit on your iPhone only holds if you can't switch it off. Blind Passcode Generator picks the Screen Time passcode for you and walks you through typing it into the phone without ever showing it to you.

HOW IT WORKS
• The app shows one key at a time. You tap it on your iPhone, then press Space.
• The real digits are mixed with extra taps that you then delete, so the keys you press don't spell out the code.
• You cover the dots on the phone and repeat a memory number in your head while you tap, which keeps the real digits from sticking.

GETTING THE CODE BACK
The code is saved on your computer behind a time-lock. To see it, your computer has to work nonstop for as long as you chose when you made it: a minute, an hour, a day. That wait is what stops you from giving in on impulse.

GOOD TO KNOW
• Built for the iPhone Screen Time passcode. Works for any keypad code of 3 to 16 digits.
• You need this computer and the iPhone side by side. Nothing is installed on the phone.
• At the end of a setup you enter your Apple ID on the phone. It is your way back in if the saved code is ever lost.
• No account, no ads, no tracking. The app makes no connections to the internet.
• Deleted codes wait 30 days in Recently deleted.
• Free and open source.

Screen Time is a feature of Apple iOS. This app is independent and is not affiliated with or endorsed by Apple Inc.
```

### Short description

```
Set an iPhone Screen Time passcode that you yourself don't know.
```

### Product features

One per box:

```
Picks a Screen Time passcode and never shows it to you
Shows one key at a time, mixed with taps you then delete
A time-lock sets how long getting the code back takes
Offers a backup to a synced folder such as OneDrive
Keeps deleted codes for 30 days
Light and dark appearance
No account, no ads, no internet connection
```

### Screenshots

The six pictures in `screenshots/`, in the order of their numbers. They are
desktop screenshots, 1520 x 1440.

### Search terms

One per box:

```
screen time passcode
screen time limit
app limits
self control
digital wellbeing
phone addiction
time lock
```

### Copyright and developer

- Copyright and trademark info: `© 2026 Sam Williams`
- Developed by: `Sam Williams`

## Submission options

### Why the package needs runFullTrust

```
Blind Passcode Generator is a packaged Win32 desktop application: a Rust program that shows its screens in WebView2. It needs runFullTrust to run as an ordinary desktop program and to keep its files in the .locker folder in the user's home folder. It declares no other restricted capability.
```

### Notes for certification

```
The app needs no account or sign-in and makes no network connections.

It helps a person set an iPhone Screen Time passcode that they don't know. It can be tested without an iPhone:

1. Choose New code, then Next, Next, Covered, Start.
2. The app shows a number to remember. Choose Begin.
3. It then shows one key at a time, such as TAP 7. A person would tap that key on the phone. For testing, just press Space each time.
4. At "Round 1 done", choose Start round 2 and keep pressing Space.
5. At "What was your number?", type the number or choose Forgot. Then choose Continue, then Done. If it offers a backup, choose Skip. Then Done.
6. The saved code is now on the first screen. Choose Reveal, then Reveal again. The time-lock runs for about one minute and then shows the code.

The cog on the first screen opens Settings. The app keeps its files in the .locker folder in the user's home folder.

Settings has a Donate button. It opens https://ko-fi.com/samwilliams123 in the user's web browser, for a voluntary donation to the developer through Ko-fi. Donating unlocks nothing: the app has no paid features. The app itself takes no payment and asks for no payment details.

Settings also has a Send feedback button. It opens a new message to blindpasscodegen@gmail.com in the user's mail app. The app sends nothing itself.
```

## Left out on purpose

- Lock text. It is an extra that is off by default, and naming it here would
  blur what the app is for.
- Store logos. Without them the Store uses the pictures in the package.
