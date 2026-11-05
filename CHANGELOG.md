# Changelog

## Unversioned

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
