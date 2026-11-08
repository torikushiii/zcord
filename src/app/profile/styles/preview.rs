use super::*;

fn preview_decoration(editor: &StylesEditor) -> Option<Option<model::AvatarDecoration>> {
    match editor.draft.decoration? {
        Some(sku_id) => {
            let item = editor
                .find_collectible(sku_id)?
                .product
                .items
                .iter()
                .find(|item| item.kind == 0)?;
            Some(Some(model::AvatarDecoration {
                asset: item.asset.clone()?,
                sku_id: Some(sku_id),
            }))
        }
        None => Some(None),
    }
}

fn preview_nameplate(editor: &StylesEditor) -> Option<Option<model::Collectibles>> {
    let nameplate = match editor.draft.nameplate? {
        Some(sku_id) => {
            let item = editor
                .find_collectible(sku_id)?
                .product
                .items
                .iter()
                .find(|item| item.kind == 2)?;
            Some(model::Nameplate {
                asset: item.asset.clone()?,
                palette: item.palette.clone(),
                label: item.label.clone(),
                sku_id: Some(sku_id),
            })
        }
        None => None,
    };
    Some(nameplate.map(|nameplate| model::Collectibles {
        nameplate: Some(nameplate),
    }))
}

pub(super) fn apply_preview(profile: &mut model::Profile, editor: &StylesEditor) {
    let draft = &editor.draft;
    let style = draft.name_style.as_ref().map(|style| {
        style.as_ref().map(|style| model::DisplayNameStyles {
            font_id: Some(style.font_id),
            effect_id: Some(style.effect_id),
            colors: style.colors.clone(),
        })
    });
    let decoration = preview_decoration(editor);
    let nameplate = preview_nameplate(editor);
    match editor.scope {
        StyleScope::Main => {
            let user = &mut profile.user.user;
            match &draft.avatar {
                Some(ImageChange::Remove) => user.avatar = None,
                Some(ImageChange::Recent { storage_hash, .. }) => {
                    user.avatar = Some(storage_hash.to_string())
                }
                _ => {}
            }
            if let Some(decoration) = decoration {
                user.avatar_decoration_data = Some(decoration);
            }
            if let Some(nameplate) = nameplate {
                user.collectibles = Some(nameplate);
            }
            if let Some(style) = style {
                user.display_name_styles = Some(style);
            }
            if let Some(accent) = draft.accent_color {
                profile.user.accent_color = accent;
                profile.user.banner_color = accent;
            }
            if matches!(draft.banner, Some(ImageChange::Remove)) {
                profile.user.banner = None;
            }
        }
        StyleScope::Guild(_) => {
            let member = profile.guild_member.get_or_insert_with(Default::default);
            if matches!(draft.avatar, Some(ImageChange::Remove)) {
                member.member.avatar = Some(None);
            }
            if let Some(decoration) = decoration {
                member.member.avatar_decoration_data = Some(decoration);
            }
            if let Some(nameplate) = nameplate {
                member.member.collectibles = Some(nameplate);
            }
            if let Some(style) = style {
                member.member.display_name_styles = Some(style);
            }
            if matches!(draft.banner, Some(ImageChange::Remove)) {
                member.banner = None;
            }
        }
    }
    let data = match editor.scope {
        StyleScope::Main => &mut profile.user_profile,
        StyleScope::Guild(_) => &mut profile.guild_member_profile,
    }
    .get_or_insert_with(Default::default);
    if let Some(colors) = draft.theme_colors {
        data.theme_colors = colors.map(Vec::from);
    }
    if let Some(accent) = draft.accent_color {
        data.accent_color = accent;
    }
    if matches!(draft.banner, Some(ImageChange::Remove)) {
        data.banner = None;
    }
    if let Some(effect) = draft.effect {
        data.profile_effect = effect.map(|sku_id| model::ProfileEffectRef { sku_id });
        data.collectibles.retain(|item| item.kind != 1);
        if let Some(sku_id) = effect {
            data.collectibles
                .push(model::ProfileCollectible { sku_id, kind: 1 });
        }
    }
    if let Some(frame) = draft.frame {
        data.collectibles.retain(|item| item.kind != 3);
        if let Some(sku_id) = frame {
            data.collectibles
                .push(model::ProfileCollectible { sku_id, kind: 3 });
        }
    }
}
