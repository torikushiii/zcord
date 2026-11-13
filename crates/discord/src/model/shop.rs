use std::collections::HashMap;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShopKind {
    Decoration,
    Effect,
    Nameplate,
    Frame,
    Bundle,
    Other,
}

impl ShopKind {
    fn from_type(kind: u64) -> Self {
        match kind {
            0 => Self::Decoration,
            1 => Self::Effect,
            2 => Self::Nameplate,
            3 => Self::Frame,
            1000 => Self::Bundle,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Decoration => "Avatar Decoration",
            Self::Effect => "Profile Effect",
            Self::Nameplate => "Nameplate",
            Self::Frame => "Profile Frame",
            Self::Bundle => "Bundle",
            Self::Other => "Item",
        }
    }

    pub fn search_type(self) -> Option<&'static str> {
        Some(match self {
            Self::Decoration => "AVATAR_DECORATION",
            Self::Effect => "PROFILE_EFFECT",
            Self::Nameplate => "NAMEPLATE",
            Self::Frame => "PROFILE_FRAME",
            Self::Bundle => "BUNDLE",
            Self::Other => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Money {
    pub amount: u64,
    pub currency: String,
    pub exponent: u8,
}

impl Money {
    pub const ORBS: &'static str = "discord_orb";

    pub fn is_orbs(&self) -> bool {
        self.currency == Self::ORBS
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShopPrices {
    pub regular: Vec<Money>,
    pub nitro: Vec<Money>,
}

impl ShopPrices {
    pub fn local(&self, nitro: bool) -> Option<&Money> {
        let tier = if nitro && !self.nitro.is_empty() {
            &self.nitro
        } else {
            &self.regular
        };
        tier.iter().find(|money| !money.is_orbs())
    }

    pub fn regular_local(&self) -> Option<&Money> {
        self.local(false)
    }

    pub fn orbs(&self) -> Option<u64> {
        self.regular
            .iter()
            .find(|money| money.is_orbs())
            .map(|money| money.amount)
    }

    pub fn is_empty(&self) -> bool {
        self.regular.is_empty() && self.nitro.is_empty()
    }
}

fn shop_prices<'de, D: Deserializer<'de>>(d: D) -> Result<ShopPrices, D::Error> {
    #[derive(Deserialize)]
    struct Price {
        #[serde(default)]
        amount: u64,
        #[serde(default)]
        currency: String,
        #[serde(default)]
        exponent: u8,
    }
    #[derive(Deserialize)]
    struct Country {
        #[serde(default, deserialize_with = "entries")]
        prices: Vec<Price>,
    }
    #[derive(Deserialize)]
    struct Tier {
        country_prices: Country,
    }
    let tiers: HashMap<String, Value> =
        serde_json::from_value(Value::deserialize(d)?).unwrap_or_default();
    let tier = |key: &str| -> Vec<Money> {
        tiers
            .get(key)
            .cloned()
            .and_then(|value| serde_json::from_value::<Tier>(value).ok())
            .map(|tier| {
                tier.country_prices
                    .prices
                    .into_iter()
                    .filter(|price| !price.currency.is_empty())
                    .map(|price| Money {
                        amount: price.amount,
                        currency: price.currency.to_ascii_lowercase(),
                        exponent: price.exponent,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(ShopPrices {
        regular: tier("0"),
        nitro: tier("4"),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ShopStyles {
    #[serde(default, rename = "background_colors", deserialize_with = "colors")]
    pub background: Vec<u32>,
    #[serde(default, rename = "button_colors", deserialize_with = "colors")]
    pub button: Vec<u32>,
}

fn colors<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u32>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|color| color.as_u64())
        .filter_map(|color| u32::try_from(color).ok())
        .map(|color| color & 0xFF_FFFF)
        .collect())
}

fn hex_color(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ShopArt {
    #[serde(default, rename = "fg_static", deserialize_with = "default_value")]
    pub foreground: Option<String>,
    #[serde(default, rename = "fg_video", deserialize_with = "default_value")]
    pub video: Option<String>,
    #[serde(default, rename = "bg_static", deserialize_with = "default_value")]
    pub background: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundledProduct {
    pub sku_id: u64,
    pub name: String,
    pub kind: ShopKind,
    pub prices: ShopPrices,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopVariant {
    pub sku_id: u64,
    pub label: String,
    pub swatch: Option<u32>,
}

#[derive(Clone)]
pub struct ShopProduct {
    pub sku_id: u64,
    pub name: String,
    pub summary: String,
    pub kind: ShopKind,
    pub premium_type: u8,
    pub collection_id: Option<u64>,
    pub unpublished_at: Option<String>,
    pub styles: ShopStyles,
    pub prices: ShopPrices,
    pub art: ShopArt,
    pub items: Vec<CollectibleItem>,
    pub bundled: Vec<BundledProduct>,
    pub variants: Vec<ShopVariant>,
}

impl ShopProduct {
    pub fn item(&self, kind: ShopKind) -> Option<&CollectibleItem> {
        self.items
            .iter()
            .find(|item| ShopKind::from_type(u64::from(item.kind)) == kind)
    }

    pub fn item_kind(item: &CollectibleItem) -> ShopKind {
        ShopKind::from_type(u64::from(item.kind))
    }
}

impl<'de> Deserialize<'de> for ShopProduct {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Bundled {
            #[serde(deserialize_with = "snowflake")]
            sku_id: u64,
            #[serde(default, deserialize_with = "default_value")]
            name: String,
            #[serde(default, rename = "type", deserialize_with = "default_value")]
            kind: u64,
            #[serde(default, deserialize_with = "shop_prices")]
            prices: ShopPrices,
        }
        #[derive(Deserialize)]
        struct Wire {
            #[serde(deserialize_with = "snowflake")]
            sku_id: u64,
            #[serde(default, deserialize_with = "default_value")]
            name: String,
            #[serde(default, deserialize_with = "default_value")]
            summary: String,
            #[serde(default, rename = "type", deserialize_with = "default_value")]
            kind: u64,
            #[serde(default, deserialize_with = "default_value")]
            premium_type: u8,
            #[serde(default, deserialize_with = "optional_id")]
            category_sku_id: Option<u64>,
            #[serde(default, deserialize_with = "default_value")]
            unpublished_at: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            styles: ShopStyles,
            #[serde(default, deserialize_with = "shop_prices")]
            prices: ShopPrices,
            #[serde(default, deserialize_with = "default_value")]
            preview_assets: Option<ShopArt>,
            #[serde(default, deserialize_with = "entries")]
            items: Vec<CollectibleItem>,
            #[serde(default, deserialize_with = "entries")]
            bundled_products: Vec<Bundled>,
        }
        let wire = Wire::deserialize(d)?;
        Ok(Self {
            sku_id: wire.sku_id,
            name: wire.name,
            summary: wire.summary,
            kind: ShopKind::from_type(wire.kind),
            premium_type: wire.premium_type,
            collection_id: wire.category_sku_id,
            unpublished_at: wire.unpublished_at,
            styles: wire.styles,
            prices: wire.prices,
            art: wire.preview_assets.unwrap_or_default(),
            items: wire.items,
            bundled: wire
                .bundled_products
                .into_iter()
                .map(|bundled| BundledProduct {
                    sku_id: bundled.sku_id,
                    name: bundled.name,
                    kind: ShopKind::from_type(bundled.kind),
                    prices: bundled.prices,
                })
                .collect(),
            variants: Vec::new(),
        })
    }
}

#[derive(Deserialize)]
struct StorefrontCollectibles {
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    kind: u64,
    #[serde(default, deserialize_with = "default_value")]
    premium_type: u8,
    #[serde(default, deserialize_with = "default_value")]
    item: Option<CollectibleItem>,
    #[serde(default, deserialize_with = "default_value")]
    option_selector_display_value: Option<String>,
}

#[derive(Default, Deserialize)]
struct StorefrontTenant {
    #[serde(default, deserialize_with = "default_value")]
    collectibles: Option<StorefrontCollectibles>,
}

#[derive(Deserialize)]
struct StorefrontOption {
    #[serde(default, deserialize_with = "default_value")]
    option_value: String,
}

#[derive(Deserialize)]
struct StorefrontSku {
    #[serde(deserialize_with = "snowflake")]
    id: u64,
    #[serde(default, deserialize_with = "default_value")]
    name: String,
    #[serde(default, deserialize_with = "default_value")]
    description: String,
    #[serde(default, deserialize_with = "shop_prices")]
    prices: ShopPrices,
    #[serde(default, deserialize_with = "default_value")]
    preview_asset_paths: Option<ShopArt>,
    #[serde(default, deserialize_with = "default_value")]
    tenant_metadata: StorefrontTenant,
    #[serde(default, deserialize_with = "entries")]
    selected_options: Vec<StorefrontOption>,
    #[serde(default, deserialize_with = "entries")]
    bundled_skus: Vec<StorefrontSku>,
}

impl StorefrontSku {
    fn collectibles(&self) -> Option<&StorefrontCollectibles> {
        self.tenant_metadata.collectibles.as_ref()
    }

    fn kind(&self) -> ShopKind {
        ShopKind::from_type(self.collectibles().map_or(u64::MAX, |c| c.kind))
    }
}

#[derive(Default, Deserialize)]
struct StorefrontProductTenant {
    #[serde(default, deserialize_with = "default_value")]
    collectibles: Option<StorefrontProductCollectibles>,
}

#[derive(Default, Deserialize)]
struct StorefrontProductCollectibles {
    #[serde(default, deserialize_with = "optional_id")]
    primary_collection_id: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    primary_collection_styles: ShopStyles,
    #[serde(default, deserialize_with = "default_value")]
    primary_collection_will_unpublish_at: Option<String>,
}

#[derive(Deserialize)]
struct StorefrontProduct {
    #[serde(deserialize_with = "snowflake")]
    id: u64,
    #[serde(default, deserialize_with = "default_value")]
    summary: String,
    #[serde(default, deserialize_with = "default_value")]
    tenant_metadata: StorefrontProductTenant,
    #[serde(default, deserialize_with = "entries")]
    skus: Vec<StorefrontSku>,
}

impl StorefrontProduct {
    fn into_shop_products(self) -> Vec<ShopProduct> {
        let collection = self.tenant_metadata.collectibles.unwrap_or_default();
        let variants: Vec<ShopVariant> = if self.skus.len() > 1 {
            self.skus
                .iter()
                .map(|sku| ShopVariant {
                    sku_id: sku.id,
                    label: sku
                        .selected_options
                        .first()
                        .map(|option| option.option_value.clone())
                        .unwrap_or_default(),
                    swatch: sku
                        .collectibles()
                        .and_then(|c| c.option_selector_display_value.as_deref())
                        .and_then(hex_color),
                })
                .collect()
        } else {
            Vec::new()
        };
        self.skus
            .into_iter()
            .map(|sku| {
                let kind = sku.kind();
                let premium_type = sku.collectibles().map_or(0, |c| c.premium_type);
                let own_item = sku
                    .tenant_metadata
                    .collectibles
                    .as_ref()
                    .and_then(|c| c.item.clone());
                let bundled: Vec<BundledProduct> = sku
                    .bundled_skus
                    .iter()
                    .map(|bundled| BundledProduct {
                        sku_id: bundled.id,
                        name: bundled.name.clone(),
                        kind: bundled.kind(),
                        prices: bundled.prices.clone(),
                    })
                    .collect();
                let items = match own_item {
                    Some(item) => vec![item],
                    None => sku
                        .bundled_skus
                        .into_iter()
                        .filter_map(|bundled| bundled.tenant_metadata.collectibles?.item)
                        .collect(),
                };
                let summary = if sku.description.is_empty() {
                    self.summary.clone()
                } else {
                    sku.description
                };
                ShopProduct {
                    sku_id: sku.id,
                    name: sku.name,
                    summary,
                    kind,
                    premium_type,
                    collection_id: collection.primary_collection_id,
                    unpublished_at: collection.primary_collection_will_unpublish_at.clone(),
                    styles: collection.primary_collection_styles.clone(),
                    prices: sku.prices,
                    art: sku.preview_asset_paths.unwrap_or_default(),
                    items,
                    bundled,
                    variants: variants.clone(),
                }
            })
            .collect()
    }
}

pub fn storefront_products(value: Value) -> Vec<ShopProduct> {
    #[derive(Deserialize)]
    struct Wire {
        #[serde(default, deserialize_with = "entries")]
        products: Vec<StorefrontProduct>,
    }
    serde_json::from_value::<Wire>(value)
        .map(|wire| {
            wire.products
                .into_iter()
                .flat_map(StorefrontProduct::into_shop_products)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone)]
pub struct ShopCollection {
    pub id: u64,
    pub name: String,
    pub summary: String,
    pub unpublished_at: Option<String>,
    pub styles: ShopStyles,
    pub hero_banner: Option<String>,
    pub hero_logo: Option<String>,
    pub catalog_banner: Option<String>,
    pub logo: Option<String>,
    pub detail_background: Option<String>,
    pub products: Vec<ShopProduct>,
    pub product_skus: Vec<u64>,
}

impl<'de> Deserialize<'de> for ShopCollection {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(deserialize_with = "snowflake")]
            sku_id: u64,
            #[serde(default, deserialize_with = "default_value")]
            name: String,
            #[serde(default, deserialize_with = "default_value")]
            summary: String,
            #[serde(default, deserialize_with = "default_value")]
            unpublished_at: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            styles: ShopStyles,
            #[serde(default, deserialize_with = "default_value")]
            hero_banner_url: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            hero_logo_url: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            catalog_banner_url: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            logo_url: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            pdp_bg_url: Option<String>,
            #[serde(default, deserialize_with = "entries")]
            products: Vec<ShopProduct>,
        }
        let wire = Wire::deserialize(d)?;
        let product_skus = wire.products.iter().map(|product| product.sku_id).collect();
        Ok(Self {
            id: wire.sku_id,
            name: wire.name,
            summary: wire.summary.trim().to_owned(),
            unpublished_at: wire.unpublished_at,
            styles: wire.styles,
            hero_banner: wire.hero_banner_url,
            hero_logo: wire.hero_logo_url,
            catalog_banner: wire.catalog_banner_url,
            logo: wire.logo_url,
            detail_background: wire.pdp_bg_url,
            products: wire.products,
            product_skus,
        })
    }
}

fn storefront_collection(value: Value) -> Option<ShopCollection> {
    #[derive(Default, Deserialize)]
    struct Meta {
        #[serde(default, deserialize_with = "default_value")]
        styles: ShopStyles,
        #[serde(default, deserialize_with = "default_value")]
        hero_banner_url: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        hero_logo_url: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        catalog_banner_url: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        logo_url: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        pdp_bg_url: Option<String>,
    }
    #[derive(Default, Deserialize)]
    struct Tenant {
        #[serde(default, deserialize_with = "default_value")]
        collectibles: Meta,
    }
    #[derive(Default, Deserialize)]
    struct Unpublish {
        #[serde(default, deserialize_with = "default_value")]
        will_unpublish_at: Option<String>,
    }
    #[derive(Deserialize)]
    struct Wire {
        #[serde(deserialize_with = "snowflake")]
        id: u64,
        #[serde(default, deserialize_with = "default_value")]
        name: String,
        #[serde(default, deserialize_with = "default_value")]
        description: String,
        #[serde(default, deserialize_with = "ids")]
        product_ids: Vec<u64>,
        #[serde(default, deserialize_with = "default_value")]
        unpublish_settings: Unpublish,
        #[serde(default, deserialize_with = "default_value")]
        tenant_metadata: Tenant,
        #[serde(default, deserialize_with = "entries")]
        products: Vec<StorefrontProduct>,
    }
    let wire: Wire = serde_json::from_value(value).ok()?;
    let mut by_product: HashMap<u64, StorefrontProduct> = wire
        .products
        .into_iter()
        .map(|product| (product.id, product))
        .collect();
    let mut products = Vec::new();
    let mut product_skus = Vec::new();
    let ordered: Vec<u64> = wire
        .product_ids
        .iter()
        .copied()
        .chain(by_product.keys().copied().collect::<Vec<_>>())
        .collect();
    for product_id in ordered {
        let Some(product) = by_product.remove(&product_id) else {
            continue;
        };
        let skus = product.into_shop_products();
        if let Some(first) = skus.first() {
            product_skus.push(first.sku_id);
        }
        products.extend(skus);
    }
    let meta = wire.tenant_metadata.collectibles;
    Some(ShopCollection {
        id: wire.id,
        name: wire.name,
        summary: wire.description.trim().to_owned(),
        unpublished_at: wire.unpublish_settings.will_unpublish_at,
        styles: meta.styles,
        hero_banner: meta.hero_banner_url,
        hero_logo: meta.hero_logo_url,
        catalog_banner: meta.catalog_banner_url,
        logo: meta.logo_url,
        detail_background: meta.pdp_bg_url,
        products,
        product_skus,
    })
}

#[derive(Clone, Default)]
pub struct ShopCollections {
    pub collections: Vec<ShopCollection>,
    pub total: u32,
}

pub fn storefront_collections(value: Value) -> ShopCollections {
    #[derive(Deserialize)]
    struct Wire {
        #[serde(default, deserialize_with = "default_value")]
        collections: Vec<Value>,
        #[serde(default, deserialize_with = "default_value")]
        total: u32,
    }
    let Ok(wire) = serde_json::from_value::<Wire>(value) else {
        return ShopCollections::default();
    };
    let collections: Vec<ShopCollection> = wire
        .collections
        .into_iter()
        .filter_map(storefront_collection)
        .collect();
    let total = wire.total.max(collections.len() as u32);
    ShopCollections { collections, total }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopHero {
    pub collection_id: Option<u64>,
    pub title: String,
    pub summary: String,
    pub skus: Vec<u64>,
    pub banner: Option<String>,
    pub logo: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopPromo {
    pub collection_id: u64,
    pub name: String,
    pub art: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopRow {
    pub title: String,
    pub collection_id: Option<u64>,
    pub skus: Vec<u64>,
    pub button: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopGrid {
    pub recommended: Vec<u64>,
    pub popular: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopInfo {
    pub title: String,
    pub body: String,
    pub banner: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShopBlock {
    Hero(ShopHero),
    Promos(Vec<ShopPromo>),
    Grid(ShopGrid),
    Info(ShopInfo),
    Row(ShopRow),
}

fn shop_block(value: Value) -> Option<ShopBlock> {
    #[derive(Deserialize)]
    struct Promo {
        #[serde(deserialize_with = "snowflake")]
        category_sku_id: u64,
        #[serde(default, deserialize_with = "default_value")]
        name: String,
        #[serde(default, deserialize_with = "default_value")]
        asset_url: Option<String>,
    }
    #[derive(Default, Deserialize)]
    struct Sorted {
        #[serde(default, deserialize_with = "ids")]
        recommended: Vec<u64>,
        #[serde(default, deserialize_with = "ids")]
        popular: Vec<u64>,
    }
    #[derive(Deserialize)]
    struct Wire {
        #[serde(rename = "type")]
        kind: u8,
        #[serde(default, deserialize_with = "optional_id")]
        category_sku_id: Option<u64>,
        #[serde(default, deserialize_with = "default_value")]
        name: String,
        #[serde(default, deserialize_with = "default_value")]
        title: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        summary: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        body: Option<String>,
        #[serde(default, deserialize_with = "ids")]
        ranked_sku_ids: Vec<u64>,
        #[serde(default, deserialize_with = "default_value")]
        sorted_sku_ids: Option<Sorted>,
        #[serde(default, deserialize_with = "default_value")]
        hero_banner_url: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        hero_logo_url: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        wide_banner_url: Option<String>,
        #[serde(default, deserialize_with = "entries")]
        subblocks: Vec<Promo>,
        #[serde(default, deserialize_with = "default_value")]
        show_button: bool,
        #[serde(default, deserialize_with = "default_value")]
        button_text: Option<String>,
    }
    let wire: Wire = serde_json::from_value(value).ok()?;
    let summary = wire
        .summary
        .map(|summary| summary.trim().to_owned())
        .unwrap_or_default();
    Some(match wire.kind {
        0 => ShopBlock::Hero(ShopHero {
            collection_id: wire.category_sku_id,
            title: wire.title.unwrap_or(wire.name),
            summary,
            skus: wire.ranked_sku_ids,
            banner: wire.hero_banner_url,
            logo: wire.hero_logo_url,
        }),
        1 => ShopBlock::Promos(
            wire.subblocks
                .into_iter()
                .map(|promo| ShopPromo {
                    collection_id: promo.category_sku_id,
                    name: promo.name,
                    art: promo.asset_url,
                })
                .collect(),
        ),
        2 => {
            let sorted = wire.sorted_sku_ids.unwrap_or_default();
            let recommended = if sorted.recommended.is_empty() {
                wire.ranked_sku_ids
            } else {
                sorted.recommended
            };
            ShopBlock::Grid(ShopGrid {
                recommended,
                popular: sorted.popular,
            })
        }
        3 => ShopBlock::Info(ShopInfo {
            title: wire.title.unwrap_or(wire.name),
            body: wire.body.unwrap_or(summary),
            banner: wire.wide_banner_url,
        }),
        4 => ShopBlock::Row(ShopRow {
            title: wire.name,
            collection_id: wire.category_sku_id,
            skus: wire.ranked_sku_ids,
            button: wire
                .button_text
                .filter(|text| wire.show_button && !text.is_empty()),
        }),
        _ => return None,
    })
}

#[derive(Clone, Default)]
pub struct ShopPage {
    pub blocks: Vec<ShopBlock>,
    pub collections: Vec<ShopCollection>,
}

impl<'de> Deserialize<'de> for ShopPage {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default, deserialize_with = "default_value")]
            shop_blocks: Vec<Value>,
            #[serde(default, deserialize_with = "entries")]
            categories: Vec<ShopCollection>,
        }
        let wire = Wire::deserialize(d)?;
        Ok(Self {
            blocks: wire
                .shop_blocks
                .into_iter()
                .filter_map(shop_block)
                .collect(),
            collections: wire.categories,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ShopSearchPage {
    #[serde(default, deserialize_with = "ids")]
    pub skus: Vec<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub pagination: ShopPagination,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ShopPagination {
    #[serde(default, deserialize_with = "default_value")]
    pub total: u32,
    #[serde(default, deserialize_with = "default_value")]
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShopSort {
    #[default]
    Popular,
    Recent,
    Oldest,
    PriceLow,
    PriceHigh,
    Relevance,
}

impl ShopSort {
    fn params(self) -> (&'static str, &'static str) {
        match self {
            Self::Popular => ("popularity", "desc"),
            Self::Recent => ("recency", "desc"),
            Self::Oldest => ("recency", "asc"),
            Self::PriceLow => ("price", "asc"),
            Self::PriceHigh => ("price", "desc"),
            Self::Relevance => ("relevance", "desc"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ShopQuery {
    pub item_types: Vec<ShopKind>,
    pub colors: Vec<&'static str>,
    pub themes: Vec<&'static str>,
    pub orbs_only: bool,
    pub collabs_only: bool,
    pub search: String,
    pub sort: ShopSort,
    pub shuffle: bool,
    pub offset: u32,
    pub limit: u32,
}

impl ShopQuery {
    pub fn params(&self) -> Vec<(&'static str, String)> {
        let mut params: Vec<(&'static str, String)> = Vec::new();
        params.extend(
            self.item_types
                .iter()
                .filter_map(|kind| kind.search_type())
                .map(|kind| ("item_types", kind.to_owned())),
        );
        params.extend(
            self.colors
                .iter()
                .map(|color| ("colors", (*color).to_owned())),
        );
        params.extend(
            self.themes
                .iter()
                .map(|theme| ("themes", (*theme).to_owned())),
        );
        if self.orbs_only {
            params.push(("orbs_eligible", "true".to_owned()));
        }
        if self.collabs_only {
            params.push(("is_first_party", "false".to_owned()));
        }
        let search: String = self.search.trim().chars().take(256).collect();
        if !search.is_empty() {
            params.push(("search", search));
        }
        params.push(("offset", self.offset.to_string()));
        params.push(("limit", self.limit.to_string()));
        if self.shuffle {
            params.push(("randomize", "true".to_owned()));
        } else {
            let (sort_type, direction) = self.sort.params();
            params.push(("sort_type", sort_type.to_owned()));
            params.push(("sort_direction", direction.to_owned()));
        }
        params.push(("published_collections_only", "true".to_owned()));
        params
    }
}
