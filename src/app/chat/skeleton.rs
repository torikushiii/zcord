use super::*;

struct Shuffle(u64);

impl Shuffle {
    fn next(&mut self, range: std::ops::Range<u32>) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        range.start + ((self.0 >> 33) as u32) % (range.end - range.start)
    }
}

const SKELETON_GROUPS: usize = 14;

pub(super) fn skeleton(seed: u64) -> impl IntoElement {
    let mut shuffle = Shuffle(seed ^ 0x9E37_79B9_7F4A_7C15);
    let bar = |width: f32, height: f32, color: Rgba| {
        div()
            .flex_none()
            .w(rpx(width))
            .h(rpx(height))
            .rounded(rpx(height / 2.))
            .bg(color)
    };
    let groups: Vec<_> = (0..SKELETON_GROUPS)
        .map(|_| {
            let name = shuffle.next(60..140) as f32;
            let lines: Vec<_> = (0..shuffle.next(1..4))
                .map(|_| {
                    let words: Vec<_> = (0..shuffle.next(2..9))
                        .map(|_| bar(shuffle.next(24..90) as f32, 14., rgba(theme::SELECTED)))
                        .collect();
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(rpx(4.))
                        .min_h(rpx(22.))
                        .children(words)
                })
                .collect();
            let image = (shuffle.next(0..6) == 0).then(|| {
                div()
                    .mt_1()
                    .w(rpx(shuffle.next(200..320) as f32))
                    .h(rpx(shuffle.next(120..200) as f32))
                    .rounded(rpx(8.))
                    .bg(rgba(theme::SELECTED))
            });
            div()
                .flex()
                .flex_none()
                .w_full()
                .mt_4()
                .py(rpx(2.))
                .pr_4()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_center()
                        .w(rpx(GUTTER))
                        .child(
                            div()
                                .mt(rpx(2.))
                                .size(rpx(40.))
                                .rounded_full()
                                .bg(rgba(theme::SELECTED)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().flex().items_center().h(rpx(22.)).child(bar(
                            name,
                            16.,
                            rgb(theme::PLACEHOLDER_STRONG),
                        )))
                        .children(lines)
                        .children(image),
                )
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .justify_end()
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_hidden()
        .pb(rpx(BOTTOM_SPACER))
        .children(groups)
}
