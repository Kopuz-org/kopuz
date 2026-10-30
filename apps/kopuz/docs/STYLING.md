# Porting Kopuz's look 1:1

The Dioxus UI is Tailwind v4 classes written inline in each component. This is
how each of them becomes a GPUI call. The source of truth is the component file
in `apps/legacy/{components,pages}/src/…`; read the classes on the element
you are porting and translate them here, rather than eyeballing a screenshot.

## The colour system

Every Kopuz theme redefines Tailwind's palette variables, so a class that looks
like a fixed colour is really a theme token:

| Class | Resolves to | GPUI |
| --- | --- | --- |
| `bg-white/N`, `text-white/N`, `border-white/N` | `--color-white`, the theme's **text** colour, at N% | `style::white(N/100., cx)` |
| `bg-black/N` | `--color-black`, the theme's **background**, at N% | `style::black(N/100., cx)` |
| `text-white` | the theme's text colour | `cx.theme().foreground` |
| `text-slate-400` | the theme's muted text | `style::slate_400(cx)` |
| `text-slate-500` | dimmer still | `style::slate_500(cx)` |
| `text-green-500` | the progress colour | `cx.theme().success` |
| `text-indigo-500` / `bg-indigo-500` | the accent | `cx.theme().primary` |
| `text-indigo-400` | accent-soft | `cx.theme().primary_hover` |
| `text-red-400` | danger | `cx.theme().danger` |
| `text-purple-600` | highlight | `cx.theme().info` |

Named constants for the alphas that recur: `style::{RAISED, HOVER, ACTIVE,
BORDER, BORDER_SOFT, TEXT_STRONG, TEXT_SECONDARY, TEXT_MUTED, TEXT_SUBTLE,
TEXT_DIM, TEXT_FAINT, SELECTED, PLAYING}`. The class you are porting may use an
alpha none of them names — `white/15`, `black/30` — and passing the literal
(`style::white(0.15, cx)`) is right; the constants exist for the levels that
recur, not as a gate. Never write a hex literal or `rgb()` in a feature crate.

Tailwind's greys beyond `slate-400`/`slate-500` (`slate-300`, the `stone-*`
family) have no Kopuz variable behind them, so pick the nearest of
`slate_400`/`slate_500`/`white(…)` and move on.

## Spacing, size and type

GPUI's scale **is** Tailwind's: `gap-3` → `.gap_3()`, `px-2` → `.px_2()`,
`h-9` → `.h_9()`, `mb-4` → `.mb_4()`, `text-xs` → `.text_xs()`, `text-sm` →
`.text_sm()`. Translate the number, not the pixels.

The type scale stops at `text_3xl`. Above it, use `.text_size(…)` with
`style::text_5xl()` / `style::text_7xl()`, or `rems(n)` for a size neither
names — `text-4xl` is `rems(2.25)`.

The scale has gaps: there is no `h_14`, `w_14`, `h_18`, … Past 12 it jumps to
16, 20, 24. For a size that is not on it, use a `style::` helper (`row_height`,
`row_cover`) or `rems(n)`, never a bare `px`.

Off-scale sizes have helpers: `text-[10px]` → `.text_size(style::text_10())`,
`text-[11px]` → `style::text_11()`.

| Class | GPUI |
| --- | --- |
| `rounded` | `.rounded(style::radius(cx))` |
| `rounded-lg` | `.rounded(style::radius_lg())` |
| `rounded-xl` | `.rounded(style::radius_xl())` |
| `rounded-full` | `.rounded_full()`, but on a `Button` it is `.rounded(cx.theme().radius_full())` — its inherent `rounded` shadows `Styled`'s |
| `text-5xl` / `text-7xl` | `.text_size(style::text_5xl())` — GPUI's scale stops at `text_3xl` |
| `truncate` | `.truncate()` (needs a bounded width: pair with `flex_1().min_w_0()`) |
| `shrink-0` | `.flex_shrink_0()` |
| `flex items-center` | `h_flex()` |
| `flex flex-col` | `v_flex()` |
| `absolute inset-0` | `.absolute().inset_0()` |
| `group` + `opacity-0 group-hover:opacity-100` | `.group("name")` + `.invisible().group_hover("name", \|s\| s.visible())` |
| `hover:bg-white/10` | `.hover(\|s\| s.bg(style::white(style::HOVER, cx)))` |
| a chip: `bg-white/10 hover:bg-white/20 text-slate-300` | `Button::…custom(ButtonCustomVariant::new(cx)…)` — `.ghost()`/`.primary()` cannot express it |
| an inverted control: `bg-white text-black` | the same, with `foreground`/`background` swapped; never a hex |

`ButtonCustomVariant` is the call this port needs most often and the one that is
easiest to miss: any Kopuz button that is not a plain ghost or the accent pill
goes through it. A `Button`'s border colour also comes from its variant, so
`bg-X border-Y` needs a custom variant too, and `rounded-full` on a `Button` is
`px(999.)` — `ButtonRounded` has no `Full`.

`Button::with_size`, `Icon::size` and `Cover::rounded` take `Pixels` only, so a
bare `px(…)` there is expected rather than a violation of the rem rule.

## Known metrics

- **Track row (desktop).** `components/src/track/row.rs` has two designs: a
  two-line Android row and the desktop column grid. The port targets the
  **desktop** one: `grid items-center h-14 p-2 rounded-lg hover:bg-white/5`,
  column template `20px | minmax(200px,2fr) | minmax(150px,1fr) |
  minmax(150px,1fr) | 64px | 40px` with a `1.5rem` column gap.
  - index `text-xs text-white/60`, replaced by a play glyph on row hover and by
    a volume glyph while that row plays;
  - title cell: cover `w-10 h-10 bg-white/5 rounded mr-4 shrink-0` (with the
    downloaded badge as a `w-3 h-3 bg-indigo-500 rounded-tl` corner on the
    cover, not an inline icon), then title `text-sm font-medium truncate`;
  - artist and album are their own columns, `text-sm text-slate-500 truncate`,
    hovering to `text-slate-400`;
  - the playing row is washed with `indigo-500 @ 12%`, a selected row with
    `white @ 7%`.
  The virtualized row height must match the row, so it is `style::row_height()`.
- Chrome icon buttons: `w-9 h-9`, `rounded-xl`, `bg-white/10`, `border-white/10`.
- Desktop sidebar: `bg-black/40`, `border-white/5`, base text `text-slate-400`.
- Cards and panels: `rounded-xl`, `bg-white/5`.

## What GPUI cannot do

Do not fake these; leave the element unstyled for that property and move on.

- `active:scale-95` and every other transform: `div` has no transform. A cover
  can still *read* as `scale-105` — see Motion below.
- `backdrop-blur-*`: no backdrop filter. Use a slightly heavier scrim instead
  and leave a one-line note.
- `shadow-[…]`: use `.shadow_lg()`/`.shadow_md()`, not a hand-built spec.
- `tracking-*`: no letter-spacing. Drop it.
- `uppercase`: no text-transform. Uppercase the string in Rust, with a
  one-line note, and remember that a translated string may not want it.
- `font-mono`: the duration column is monospaced upstream; there is no mono
  family wired up here, so it renders in the UI font.
- **Do not wrap a popup trigger in `.invisible()` or `.opacity(0.)`.** `Div::paint`
  returns early when hidden and takes the deferred popup content with it, so an
  open menu vanishes the moment the pointer leaves the element that revealed it.
  A hover-revealed trigger has to track whether its own popup is open and only
  hide itself while it is closed — `track_list`'s row menu is the worked example.
- **A `Button` sizes its glyph, not you.** `with_size` sizes the *box* and the
  glyph silently becomes ¾ of it; there is no public `icon_size`. To get a
  Tailwind glyph size, ask for `with_size(glyph * 4 / 3)` and restate the box
  with `.size(box)`, which wins because the instance style is refined last.
  Miss this and every icon in the app is drawn about twice its intended size.
- A `Button`'s own hover treatment is applied after yours, so a Tailwind hover
  cannot be ported onto one. Where the original is a bare element with a hover
  colour — a chip, a chevron, a close ✕ — build it from an interactive `div`
  rather than reaching for `Button`.
  But a Button's colours *can* still be tweened. `Button::on_hover(cx.listener(..))`
  is the sanctioned way to drive a tween from a Button's own hover, and when you
  do, pass the tweened colour to **both** `.color()` and `.hover()` on its
  `ButtonCustomVariant`. It is only the `.hover(|…| …)` closure that cannot
  reach a Button.
  An instance `.bg()` / `.text_color()` is **not** a second way, though it looks
  like one: `render` ends in `refine_style(&instance_style)`, which refines the
  *base* style, while its `.hover()` fills a separate layer GPUI applies over
  the base at paint time. So an instance tween is overpainted for exactly as
  long as the pointer is down on the element — the one state you were animating.
  It survives only where `render` skips the hover branch altogether, which is
  `disabled` or `selected` (button.rs:672). That carve-out is why the side panel
  tabs get away with it: they tween on *selection*, and a selected Button has no
  hover layer. Tween anything else through the variant or an owning frame.
- `Progress` paints its track as its own bar colour at 20%; an original with a
  separate track colour cannot be matched without hand-rolling the bar.
- A rem helper cannot be handed to `Cover::size`, `Icon::size` or anything else
  taking `Pixels`; wrap it in `style::to_px(measure, window)`.

Two colours the table does not list: the playback-error banner is
`rose-500` at 15/20/70% — use `cx.theme().danger` at those alphas — and the
scan and connecting states are amber, which is `cx.theme().warning`.

## Skins

A skin is metrics and chrome proportions; a palette is colour, and the two stay
orthogonal. `components::skin` is the registry: a skin implements the `Skin` trait,
returning a `Metrics` and a `Chrome` it may fill field by field from
`Metrics::default()`, plus optional asset swaps. `Normal` is the only one
registered and is the baseline every value above is measured in.

`layout.skin` names the skin; `settings_interface::apply` resolves it once per
change into the registry and hands the component theme its `radius` /
`radius_lg`. Nothing above reads it: the helpers in `components::style` already
answer with the active skin's numbers, which is why `style::radius_lg()`,
`style::row_height()`, `style::media_row()`, `style::density_row(..)` and
`style::chrome()` exist. A radius, a row height, a control height or a bar's
own proportion is one of those calls, never a literal at the call site. Vaxry
is not ported; it lands as another `impl Skin`, not as a branch.

## The `.hover()` panic

`Button::render` sets its own hover style, and GPUI's `hover()` carries a
`debug_assert!("hover style already set")`. So **`.hover(|…| …)` on a `Button`
panics the moment that element renders** — in a debug build it takes the whole
app down, and it only fires when you visit the page that draws it.

A Button's colours come from `ButtonCustomVariant::new(cx).color(..).hover(..)
.active(..).foreground(..)` instead. `.hover(colour)` on a *variant* is fine;
it is the closure form on the button that is fatal. A plain `div()` may of
course take one closure hover.

Before finishing, check your crate:

```sh
grep -rn "\.hover(|\|\.hover(move |" <yours>/src
```

and confirm each receiver is a `div`/`h_flex`/`v_flex`, not a `Button` or a
helper that returns one.

## Motion

`transition-*` and `duration-*` **do** port. `gpui_base::motion` has a real
transition system and `components::motion` wraps it with Kopuz's timings. It is
keyed by element id, honours reduce-motion for free (which is what
`cfg.reduce_animations` should drive), and interpolates anything `Lerp`:
`f32`, `Pixels`, `Point<Pixels>` and `Hsla`.

The shape is: compute the value you *want* for the current state, hand it to
`motion`, and style with what comes back.

```rust
// `transition-colors` on a hover: ask for the target, paint the tween.
let target = if hovered { style::white(style::HOVER, cx) } else { style::white(style::RAISED, cx) };
let background = motion::color(("row-bg", row_id), target, window, cx);
div().bg(background)
```

| Class | GPUI |
| --- | --- |
| `transition-colors` | `motion::color(id, target, window, cx)` |
| `duration-300` on a colour | `motion::color_over(id, target, motion::SLOW, window, cx)` |
| `group-hover:scale-105` on a cover | `motion::zoom(id, base_size, hovered, window, cx)`, inside an `overflow_hidden` frame |
| a fading opacity | `motion::amount(id, target, motion::FAST, window, cx)` |

Two rules. The id must be stable and unique per value — `("card-cover", album.id)`,
never a list index — because it keys the channel that holds the tween's state.
And a transition needs the state it reacts to: a hover tween wants the element's
own hover state, so it is `div().id(..)` plus tracked hover, not a `.hover()`
refinement.

What still cannot move: anything needing a transform (`rotate`, `translate`,
a real `scale` on a `div`), and `active:scale-95` press feedback.
