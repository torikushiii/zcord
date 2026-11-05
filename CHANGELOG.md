# Changelog

## Unversioned

## 0.5.0

- Major: Go Live now has Stream Mode presets (Gaming, Screenshare, Custom) and streams up to 1440p at 60fps or at source resolution, matching your Nitro or server boost perks.
- Major: Go Live now streams your computer's sound, without zcord's own audio.
- Major: Added a streaming panel above Voice Connected showing what you share, its quality and who is watching.
- Major: Added Input Options and Output Options menus beside the mute and deafen buttons, with device, input profile and volume controls.
- Major: Message requests now open in a sidebar beside the requests list.
- Minor: Added the Friend Suggestions list to the Friends page.
- Minor: The stream menu can now stop the stream, change what is shared, switch quality and toggle stream audio while live.
- Minor: Stream previews are now uploaded, so others see a thumbnail of your stream.
- Minor: The share picker now shows app icons, and hovering a source shows a Share Screen button.
- Minor: Added Mute stream audio and Hide stream preview options to the stream settings.
- Minor: Added a right-click menu to the message box with the Send Message Button option and Paste.
- Minor: Audio devices are now listed like Discord, with their type icon and connection name.
- Minor: Added Input Profile presets (Voice Isolation, Studio, Custom).
- Minor: Hovering a standard emoji in a message now shows its name, and clicking it shows what it is.
- Minor: Emoji autocomplete now shows each emoji's server and fits the window height.
- Minor: The home button badge now counts pending friend and message requests.
- Minor: Clicking a voice channel you aren't in now only joins it; click it again to open the call.
- Bugfix: Fixed blurry, low frame rate streams.
- Bugfix: Fixed streams and voice cutting out for several seconds when someone joins the call.
- Bugfix: Fixed stream bandwidth estimates being misread, which capped stream bitrate.
- Bugfix: Fixed standard emoji looking jagged.
- Bugfix: Fixed the audio device dropdowns in Voice & Video settings not opening.
- Bugfix: Fixed Windows notifications not showing after installing with the installer.
- Bugfix: Fixed tooltip arrows pointing at the wrong button near the window edge.
- Bugfix: Fixed "Not Sharing" showing in the user panel while not invisible.
- Dev: The Discord protocol now lives in its own crate, `zcord-discord`.
- Dev: Screen share pixel conversion moved to the `zcord-convert` crate so dev builds can optimize it.
- Dev: CI now builds and tests the whole workspace.
- Dev: Read acks are now sent three seconds after reading, as Discord's client does.
- Dev: Stream pipeline stats are logged at debug level under `zcord_discord::voice::stream`.

## 0.4.0

- Major: Hovering a reaction now shows who reacted, and a new View Reactions dialog lists everyone who reacted with each emoji, super reactions included. Open it from the message menu or from "N others" in the tooltip.
- Major: Your display name, pronouns and server tag can now be edited in place in the profile editor and are saved with the rest of your profile.
- Minor: Profile effects now play using a few megabytes of memory instead of hundreds.
- Minor: Animations that have been off screen for 30 seconds are released from memory.
- Minor: The server tag picker matches Discord's size and colors, and its list scrolls.
- Minor: The Edit Profile icon in the account menu is smaller, matching Discord.
- Minor: The Show Styles hint uses the app's own tooltip.
- Minor: Several profile editor surfaces now follow the light theme.
- Bugfix: Fixed profile effects showing leftover or doubled sprites.
- Bugfix: Fixed the avatar decoration preview not animating.
- Bugfix: Fixed the user panel showing activity set by apps as a game being played; it now shows only games detected running.
- Bugfix: Fixed a crash when hovering a forum's New Post button while posting was blocked.
- Bugfix: Fixed the typing indicator covering the "older messages" button.
- Bugfix: Fixed the emoji, GIF and sticker pickers opening without their search box focused.
- Bugfix: Fixed Escape closing the whole profile editor instead of the open picker or dialog.
- Dev: Profile effects now play through the GIF streaming player.
- Dev: APNG frames are now composited by zcord itself, working around a disposal bug in the `image` crate.
- Dev: Decoded frame cache files are read without a full in-memory copy, and the disk cache now counts streamed frame spools.
