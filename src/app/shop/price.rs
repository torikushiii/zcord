use crate::discord::model::{Money, ShopKind, ShopPrices, ShopProduct};

const NITRO: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub text: String,
    pub orbs: bool,
    pub nitro: bool,
    pub discount: Option<u32>,
}

pub(super) fn has_nitro_pricing(premium_type: Option<u8>) -> bool {
    premium_type == Some(NITRO)
}

pub(super) fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn symbol(currency: &str) -> Option<&'static str> {
    Some(match currency {
        "usd" => "$",
        "eur" => "€",
        "gbp" => "£",
        "jpy" => "¥",
        "inr" => "₹",
        "krw" => "₩",
        "brl" => "R$",
        "cad" => "CA$",
        "aud" => "A$",
        "nzd" => "NZ$",
        "mxn" => "MX$",
        "hkd" => "HK$",
        "twd" => "NT$",
        "cny" => "CN¥",
        "ils" => "₪",
        "php" => "₱",
        "vnd" => "₫",
        _ => return None,
    })
}

pub(super) fn money(money: &Money, always_decimals: bool) -> String {
    let scale = 10u64.pow(u32::from(money.exponent.min(6)));
    let whole = grouped(money.amount / scale);
    let fraction = money.amount % scale;
    let number = if money.exponent == 0 || (fraction == 0 && !always_decimals) {
        whole
    } else {
        format!(
            "{whole}.{fraction:0width$}",
            width = usize::from(money.exponent.min(6))
        )
    };
    match symbol(&money.currency) {
        Some(symbol) => format!("{symbol}{number}"),
        None => format!("{}\u{a0}{number}", money.currency.to_ascii_uppercase()),
    }
}

fn bundle_discount(
    product: &ShopProduct,
    price: u64,
    pick: impl Fn(&ShopPrices) -> Option<u64>,
) -> Option<u32> {
    if product.kind != ShopKind::Bundle || product.bundled.is_empty() || price == 0 {
        return None;
    }
    let total = product
        .bundled
        .iter()
        .map(|bundled| pick(&bundled.prices))
        .sum::<Option<u64>>()?;
    if total <= price {
        return None;
    }
    let percent = ((total - price) as f64 / total as f64 * 100.).round() as u32;
    (percent > 0).then_some(percent)
}

pub(super) fn card(product: &ShopProduct, nitro: bool) -> Option<Shown> {
    if let Some(orbs) = product.prices.orbs() {
        return Some(Shown {
            text: grouped(orbs),
            orbs: true,
            nitro: false,
            discount: bundle_discount(product, orbs, ShopPrices::orbs),
        });
    }
    let price = product.prices.local(nitro)?;
    Some(Shown {
        text: money(price, false),
        orbs: false,
        nitro: nitro && !product.prices.nitro.is_empty(),
        discount: discount(product, price),
    })
}

pub(super) fn discount(product: &ShopProduct, price: &Money) -> Option<u32> {
    bundle_discount(product, price.amount, |prices| {
        prices
            .regular_local()
            .filter(|regular| regular.currency == price.currency)
            .map(|regular| regular.amount)
    })
}

pub(super) fn nitro_offer(product: &ShopProduct, nitro: bool) -> Option<String> {
    if nitro {
        return None;
    }
    let regular = product.prices.regular_local()?;
    let member = product.prices.local(true)?;
    (member.amount < regular.amount && member.currency == regular.currency)
        .then(|| money(member, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::model::{BundledProduct, ShopArt, ShopStyles};

    fn idr(amount: u64) -> Money {
        Money {
            amount,
            currency: "idr".into(),
            exponent: 2,
        }
    }

    fn orbs(amount: u64) -> Money {
        Money {
            amount,
            currency: Money::ORBS.into(),
            exponent: 0,
        }
    }

    fn product(kind: ShopKind, regular: Vec<Money>, nitro: Vec<Money>) -> ShopProduct {
        ShopProduct {
            sku_id: 1,
            name: "Sample".into(),
            summary: String::new(),
            kind,
            premium_type: 0,
            collection_id: None,
            unpublished_at: None,
            styles: ShopStyles::default(),
            prices: ShopPrices { regular, nitro },
            art: ShopArt::default(),
            items: Vec::new(),
            bundled: Vec::new(),
            variants: Vec::new(),
        }
    }

    #[test]
    fn money_drops_zero_cents_on_cards_and_keeps_them_on_buttons() {
        assert_eq!(money(&idr(13_000_000), false), "IDR\u{a0}130,000");
        assert_eq!(money(&idr(13_000_000), true), "IDR\u{a0}130,000.00");
        let usd = Money {
            amount: 699,
            currency: "usd".into(),
            exponent: 2,
        };
        assert_eq!(money(&usd, false), "$6.99");
        let yen = Money {
            amount: 1200,
            currency: "jpy".into(),
            exponent: 0,
        };
        assert_eq!(money(&yen, true), "¥1,200");
        assert_eq!(grouped(158_170), "158,170");
        assert_eq!(grouped(0), "0");
    }

    #[test]
    fn cards_prefer_orbs_then_the_nitro_price() {
        let mut item = product(
            ShopKind::Decoration,
            vec![idr(3_950_000), orbs(4100)],
            vec![idr(3_300_000), orbs(4100)],
        );
        let shown = card(&item, true).unwrap();
        assert_eq!(
            (shown.text.as_str(), shown.orbs, shown.nitro),
            ("4,100", true, false)
        );
        item.prices.regular.pop();
        let shown = card(&item, true).unwrap();
        assert_eq!(
            (shown.text.as_str(), shown.nitro),
            ("IDR\u{a0}33,000", true)
        );
        let shown = card(&item, false).unwrap();
        assert_eq!(
            (shown.text.as_str(), shown.nitro),
            ("IDR\u{a0}39,500", false)
        );
        assert_eq!(
            nitro_offer(&item, false).as_deref(),
            Some("IDR\u{a0}33,000")
        );
        assert_eq!(nitro_offer(&item, true), None);
    }

    #[test]
    fn bundle_discount_compares_with_regular_item_prices() {
        let mut bundle = product(
            ShopKind::Bundle,
            vec![idr(16_000_000)],
            vec![idr(13_000_000)],
        );
        bundle.bundled = (0..3)
            .map(|index| BundledProduct {
                sku_id: index,
                name: String::new(),
                kind: ShopKind::Decoration,
                prices: ShopPrices {
                    regular: vec![idr(7_800_000)],
                    nitro: vec![idr(5_200_000)],
                },
            })
            .collect();
        assert_eq!(card(&bundle, true).unwrap().discount, Some(44));
        assert_eq!(card(&bundle, false).unwrap().discount, Some(32));
        bundle.bundled[0].prices.regular.clear();
        assert_eq!(card(&bundle, false).unwrap().discount, None);
    }
}
