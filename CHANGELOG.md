# Changelog

## Unversioned

## 0.6.0

- Major: Added the Quests page with the sponsored banner, quest cards and the All Quests and Claimed Quests tabs. Quests can't be started from zcord yet.
- Major: Streams can now be popped out into their own window, which can be pinned on top of other windows.
- Major: While streaming you can now watch a preview of your own stream.
- Major: Embeds now follow Discord's layout, including portrait YouTube and TikTok videos.
- Major: The image viewer can now zoom, pan a zoomed image by dragging, and page through every image in a message.
- Major: Right-clicking an image in the image viewer now offers Copy Image, Save Image, Copy Image Link and Open Image Link.
- Major: Right-clicking someone in the member list now offers Profile, Mention, Message, Ignore, Block and Copy User ID.
- Major: Messages from blocked users now collapse behind "N blocked messages — Show", and a blocked user's profile asks before it is shown.
- Minor: Direct messages with unread messages now show at the top of the server rail.
- Minor: Calls now show in chat with how long they lasted, and ongoing calls have a link to join.
- Minor: Added an audio menu to the call view with device and stream volume controls.
- Minor: The bar above the message box now also shows when older messages fail to load, with a Try Again button, and when you are viewing a reply, with a button to jump back.
- Minor: The send button now fills in once there is something to send.
- Minor: Activity cards are darker, show New Player and Trending badges, and link to profiles.
- Minor: Direct messages with blocked users are faded in the list and show a blocked icon, and their message box is replaced by an Unblock bar.
- Minor: Messages that can't be delivered now explain why.
- Minor: Copy ID menu items now use an ID card icon.
- Bugfix: Fixed text selections being lost when other parts of the window redrew.
- Bugfix: Fixed starting a DM call sometimes not ringing the other person.
- Bugfix: Fixed the Streaming Now card staying open after switching servers.
- Bugfix: Fixed the server name tooltip flickering when a server folder expands or collapses.
- Dev: Credited the video slash icon in the icon licence.

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
