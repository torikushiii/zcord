use super::*;

pub(super) fn name_style(
    font_id: Option<u32>,
    effect_id: Option<u32>,
    colors: &[u32],
) -> NameStyle {
    NameStyle {
        font_id: font_id.unwrap_or(11),
        effect_id: effect_id.filter(|id| (1..=8).contains(id)).unwrap_or(1),
        colors: colors.to_vec(),
    }
}

fn profile_values(profile: Option<&model::UserProfile>) -> StyleValues {
    let Some(profile) = profile else {
        return StyleValues::default();
    };
    StyleValues {
        banner: profile.banner.clone().map(Into::into),
        effect: profile
            .profile_effect
            .as_ref()
            .map(|effect| effect.sku_id)
            .or_else(|| {
                profile
                    .collectibles
                    .iter()
                    .find(|item| item.kind == 1)
                    .map(|item| item.sku_id)
            }),
        frame: profile
            .collectibles
            .iter()
            .find(|item| item.kind == 3)
            .map(|item| item.sku_id),
        theme_colors: profile
            .theme_colors
            .as_deref()
            .and_then(|colors| colors.try_into().ok()),
        accent_color: profile.accent_color,
        ..Default::default()
    }
}

pub(super) fn main_values(profile: &model::Profile) -> StyleValues {
    let user = &profile.user.user;
    let mut values = profile_values(profile.user_profile.as_ref());
    values.avatar = user.avatar.clone().map(Into::into);
    values.banner = profile
        .user
        .banner
        .clone()
        .map(Into::into)
        .or(values.banner);
    values.accent_color = profile
        .user
        .accent_color
        .or(profile.user.banner_color)
        .or(values.accent_color);
    values.decoration = user
        .avatar_decoration_data
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(|decoration| decoration.sku_id);
    values.nameplate = user
        .collectibles
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(|items| items.nameplate.as_ref())
        .and_then(|nameplate| nameplate.sku_id);
    values.name_style = user
        .display_name_styles
        .as_ref()
        .and_then(Option::as_ref)
        .map(|style| name_style(style.font_id, style.effect_id, &style.colors));
    values
}

pub(super) fn guild_values(profile: &model::Profile) -> StyleValues {
    let mut values = profile_values(profile.guild_member_profile.as_ref());
    if let Some(member) = &profile.guild_member {
        values.avatar = member
            .member
            .avatar
            .as_ref()
            .and_then(Option::as_ref)
            .cloned()
            .map(Into::into);
        values.banner = member.banner.clone().map(Into::into).or(values.banner);
        values.decoration = member
            .member
            .avatar_decoration_data
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(|decoration| decoration.sku_id);
        values.nameplate = member
            .member
            .collectibles
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(|items| items.nameplate.as_ref())
            .and_then(|nameplate| nameplate.sku_id);
        values.name_style = member
            .member
            .display_name_styles
            .as_ref()
            .and_then(Option::as_ref)
            .map(|style| name_style(style.font_id, style.effect_id, &style.colors));
    }
    values
}

fn normalize<T: PartialEq>(change: &mut Option<Option<T>>, saved: &Option<T>) {
    if change.as_ref() == Some(saved) {
        *change = None;
    }
}

fn normalize_image(change: &mut Option<ImageChange>, saved: &Option<SharedString>) {
    let unchanged = match change.as_ref() {
        Some(ImageChange::Remove) => saved.is_none(),
        Some(ImageChange::Recent { storage_hash, .. }) => saved.as_ref() == Some(storage_hash),
        _ => false,
    };
    if unchanged {
        *change = None;
    }
}

pub(super) fn normalize_draft(draft: &mut StyleDraft, saved: &StyleValues) {
    if let Some(Some(style)) = &mut draft.name_style
        && !(1..=8).contains(&style.effect_id)
    {
        style.effect_id = 1;
    }
    normalize_image(&mut draft.avatar, &saved.avatar);
    normalize_image(&mut draft.banner, &saved.banner);
    normalize(&mut draft.decoration, &saved.decoration);
    normalize(&mut draft.nameplate, &saved.nameplate);
    normalize(&mut draft.effect, &saved.effect);
    normalize(&mut draft.frame, &saved.frame);
    normalize(&mut draft.name_style, &saved.name_style);
    normalize(&mut draft.theme_colors, &saved.theme_colors);
    normalize(&mut draft.accent_color, &saved.accent_color);
}

pub(super) fn style_patch(draft: &StyleDraft, saved: &StyleValues) -> model::ProfileStylePatch {
    let mut patch = model::ProfileStylePatch {
        avatar: match &draft.avatar {
            Some(ImageChange::Upload { data_uri, .. }) => Some(Some(data_uri.to_string())),
            Some(ImageChange::Remove) => Some(None),
            _ => None,
        },
        banner: match &draft.banner {
            Some(ImageChange::Upload { data_uri, .. }) => Some(Some(data_uri.to_string())),
            Some(ImageChange::Remove) => Some(None),
            _ => None,
        },
        decoration: draft.decoration,
        nameplate: draft.nameplate,
        theme_colors: draft.theme_colors,
        accent_color: draft.accent_color,
        ..Default::default()
    };
    if let Some(style) = &draft.name_style {
        match style {
            Some(style) => {
                if saved
                    .name_style
                    .as_ref()
                    .is_none_or(|saved| saved.font_id != style.font_id)
                {
                    patch.display_name_font_id = Some(Some(style.font_id));
                }
                let effect = name_style(None, Some(style.effect_id), &[]).effect_id;
                if saved
                    .name_style
                    .as_ref()
                    .is_none_or(|saved| saved.effect_id != effect)
                {
                    patch.display_name_effect_id = Some(Some(effect));
                }
                if saved
                    .name_style
                    .as_ref()
                    .is_none_or(|saved| saved.colors != style.colors)
                {
                    patch.display_name_colors = Some(Some(style.colors.clone()));
                }
            }
            None => {
                patch.display_name_font_id = Some(None);
                patch.display_name_effect_id = Some(None);
                patch.display_name_colors = Some(None);
            }
        }
    }
    if draft.effect.is_some() || draft.frame.is_some() {
        patch.collectibles_sku_ids = Some(
            draft
                .effect
                .unwrap_or(saved.effect)
                .into_iter()
                .chain(draft.frame.unwrap_or(saved.frame))
                .collect(),
        );
    }
    patch
}

pub(super) fn style_error(error: &Error) -> SharedString {
    match error {
        Error::Rejected { message, .. } if !message.is_empty() => {
            message.chars().take(240).collect::<String>().into()
        }
        _ => error.to_string().into(),
    }
}
