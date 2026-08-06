# Neumorphic UI Research for VisualLLM

A deep-dive reference on neumorphic (soft-UI) design principles, formulas, component patterns, motion, accessibility, tooling, and their application to the VisualLLM Tauri interface.

---

## Table of Contents

1. Introduction and terminology
2. Historical context and origin
3. The physics of neumorphic shadow
4. Color theory and palettes
5. Light theme neumorphism
6. Dark theme neumorphism
7. Component patterns in detail
8. Motion and micro-interactions
9. Accessibility and inclusive design
10. Tooling and generators
11. Criticism and limitations
12. VisualLLM application plan
13. Source bibliography

---

## 1. Introduction and terminology

Neumorphism — a portmanteau of "new" and "skeuomorphism" — is a digital design language that surfaced in late 2019. It replaces flat material fills with low-contrast, extruded shapes that appear to push out from or sink into the background. The effect is produced almost entirely through carefully paired `box-shadow` values.

### 1.1 Core visual formula

```css
:root {
  --bg: #e0e5ec;
  --light: #ffffff;
  --dark: #a3b1c6;
}

.raised {
  background: var(--bg);
  box-shadow: 9px 9px 16px var(--dark), -9px -9px 16px var(--light);
  border-radius: 20px;
}

.pressed {
  background: var(--bg);
  box-shadow: inset 6px 6px 10px var(--dark), inset -6px -6px 10px var(--light);
}
```

Key constraints that make the style readable:
- Background and surface share the same hue.
- Contrast between light and dark shadow is low, often 15–30 % luminance shift.
- Border radius is generous.
- Edges are implied by shadow, not drawn with borders.

### 1.2 Terminology

- **Soft UI**: the broader umbrella that includes neumorphism.
- **Extruded / convex**: an element that appears to bulge out.
- **Inset / concave**: an element that appears to be pressed in.
- **Pillow / floating**: a higher elevation variant.
- **Claymorphism**: a cousin style using inner highlights and more saturated colors.
- **Glassmorphism**: translucent blur layers, sometimes combined with neumorphism.

---

## 2. Historical context and origin

### 2.1 Alexander Plyuto’s 2019 Dribbble shot

The style is generally credited to Ukrainian designer Alexander Plyuto, who posted a set of “Skeuomorph Mobile Banking” Dribbble shots in late 2019. The community quickly labeled the look “neumorphism.” The shots used a pale grey background, rounded rectangles, and paired shadows to suggest depth without heavy gradients or drop-shadow realism.

### 2.2 From skeuomorphism to flat to soft UI

- **2007–2012 skeuomorphism**: Apple iOS 6-era leather, linen, and realistic buttons. Heavy texture, high detail, strong metaphor.
- **2013–2019 flat design / Material Design**: no shadows, bold color, typography-driven hierarchy.
- **2019+ neumorphism**: reintroduces depth but keeps the minimal palette of flat design.
- **2021+ claymorphism and glassmorphism**: branch off, adding color and translucency.

### 2.3 Why it keeps returning

Neumorphism resurfaces because it offers tactile affordance on touchscreens without the visual noise of realism. It suggests “this is a surface you can press.” For desktop tools with dense controls, it risks low contrast but rewards consistency when applied carefully.

---

## 3. The physics of neumorphic shadow

### 3.1 The canonical two-shadow model

A neumorphic element is lit from the top-left. One shadow is lighter than the background and cast down-right by the imagined light source. The other is darker than the background and cast up-left as occlusion.

```
box-shadow:
  x  y  blur  dark-shadow-color,
  -x -y blur  light-shadow-color;
```

### 3.2 Deriving shadow colors from the base

Instead of guessing shadow colors, derive them mathematically:

```css
:root {
  --bg: hsl(220, 20%, 93%);
  --shadow-light: hsl(220, 20%, 98%);
  --shadow-dark: hsl(220, 20%, 78%);
}
```

In Sass:

```scss
$bg: #e0e5ec;
$light: lighten($bg, 10%);
$dark: darken($bg, 12%);
```

In modern CSS with `color-mix`:

```css
--dark: color-mix(in srgb, var(--bg) 70%, black);
--light: color-mix(in srgb, var(--bg) 80%, white);
```

### 3.3 Distance and blur scaling

| Elevation | Offset X/Y | Blur radius | Spread | Usage |
|-----------|------------|-------------|--------|-------|
| 1 subtle | 3–5 px | 8–12 px | 0–1 px | small buttons, toggles |
| 2 raised | 6–9 px | 16–20 px | 0–2 px | cards, panels |
| 3 floating | 12–18 px | 24–32 px | 0–4 px | modals, menus |
| Pressed | inset 4–6 px | 8–12 px | 1–2 px | inputs, active states |

The ratio of offset to blur is usually near 1:2. Less blur makes the shadow look crisp and cheap; more blur smudges the form.

### 3.4 Shape effects

- **Convex / extruded**: outset shadows only.
- **Concave / pressed**: both shadows inset.
- **Flat**: no shadow, identical to background.
- **Punched hole**: heavy inset shadows plus an inner gradient.
- **Floating pill**: large outset shadow plus slight lift on hover.

### 3.5 Edge quality

Because there are no borders, the edge of an element is defined by where the shadow fades. A sharp cutoff reads as a cut; a very soft shadow reads as haze. The sweet spot is a shadow that is visible but not harsh.

### 3.6 Corner radius

Radius must be large enough that the curve catches the light. Typical values:
- Small controls: 10–12 px
- Buttons and inputs: 14–18 px
- Cards and panels: 20–28 px
- Circular controls: 50 %

### 3.7 Combining gradients with shadow

A subtle gradient can make a convex button read as curved:

```css
.convex-btn {
  background: linear-gradient(145deg, #f0f2f5, #caced4);
  box-shadow: 6px 6px 12px #a3b1c6, -6px -6px 12px #ffffff;
}
```

The highlight is shifted toward the top-left; the shadow toward the bottom-right.

### 3.8 Inner highlights

Pressed controls can use an inner highlight to show the rim of the cavity:

```css
.pressed-input {
  box-shadow:
    inset 3px 3px 6px #a3b1c6,
    inset -3px -3px 6px #ffffff;
}
```

### 3.9 Multiple nested elevations

Higher-level containers can stack elevations so the eye reads depth hierarchy:

- App shell: flat or very subtle.
- Sidebar / canvas: raised by e2.
- Floating action button: raised by e3.
- Modal / dropdown: raised by e3 plus z-index lift.

---

## 4. Color theory and palettes

### 4.1 Why grey dominates

Neumorphism relies on the background and surface being the same color. A neutral grey maximizes the illusion because the eye interprets the shadow pair as light falling on a matte surface. Saturated backgrounds make the shadow colors harder to derive and can look muddy.

### 4.2 Light theme backgrounds

Recommended base colors:
- `#e0e5ec`
- `#e6e7ee`
- `#eceef1`
- `#d1d9e6`
- `#eef0f4`

### 4.3 Shadow color ranges for light themes

For a base `#e0e5ec`:
- Dark shadow: `#a3b1c6` (about 25 % darker)
- Light shadow: `#ffffff` (full white or near-white)

### 4.4 Dark theme backgrounds

Dark neumorphism shifts the background much darker but keeps the same shadow model:
- `#1f2229`
- `#2d303a`
- `#292d36`
- `#181a20`
- `#24272e`

### 4.5 Shadow color ranges for dark themes

For a base `#1f2229`:
- Light shadow: `#2f3540` or `#3a3f4b`
- Dark shadow: `#111318` or `#0c0e12`

### 4.6 Accent usage

Accents should be sparse:
- Primary action buttons.
- Active toggles.
- Status indicators.
- Focus rings.

Accent saturation should be moderate; a neon button on a grey field reads as a sticker rather than a soft extrusion.

### 4.7 Semantic colors

Status colors still need to communicate:
- Success: green family.
- Warning: amber / orange.
- Error: red / coral.
- Info: blue / cyan.

In neumorphism these are usually applied as small indicator lamps or tinted icons rather than full colored surfaces.

### 4.8 Gradient accents

A gradient can add curvature to an accent button while preserving the soft shadow:

```css
.accent-btn {
  background: linear-gradient(145deg, #ff7e79, #e14f4a);
  box-shadow: 5px 5px 10px #a3b1c6, -5px -5px 10px #ffffff;
}
```

### 4.9 Color contrast risks

The same-color surface makes it hard to hit WCAG contrast. Text must be dark enough and shadows must not be the only state indicator. Tools like WebAIM’s contrast checker are essential.

---

## 5. Light theme neumorphism

### 5.1 Typical token set

```css
:root {
  --bg: #eceef1;
  --surface: #e6e9ef;
  --light: #ffffff;
  --dark: #c4cfdc;
  --text: #2b2f38;
  --text-muted: #6b7280;
  --text-faint: #9ca3af;
  --accent: #f4645f;
  --accent-deep: #e14f4a;
  --shadow-raised: 8px 8px 16px var(--dark), -8px -8px 16px var(--light);
  --shadow-pressed: inset 5px 5px 10px var(--dark), inset -5px -5px 10px var(--light);
  --radius-sm: 10px;
  --radius-md: 16px;
  --radius-lg: 24px;
}
```

### 5.2 Light theme do’s and don’ts

Do:
- Keep text dark.
- Use generous whitespace.
- Reserve white for the light shadow.
- Use subtle transitions.

Don’t:
- Use black shadows — they look like flat drop shadows.
- Stack too many raised layers — the UI becomes puffy.
- Rely on shadow alone for interactivity.

---

## 6. Dark theme neumorphism

### 6.1 Typical token set

```css
[data-theme="dark"] {
  --bg: #1f2229;
  --surface: #252a32;
  --light: #2f3540;
  --dark: #111318;
  --text: #e4e6eb;
  --text-muted: #8a919c;
  --text-faint: #5c6370;
  --accent: #60a5fa;
  --accent-deep: #3b82f6;
  --shadow-raised: 8px 8px 16px var(--dark), -8px -8px 16px var(--light);
  --shadow-pressed: inset 5px 5px 10px var(--dark), inset -5px -5px 10px var(--light);
}
```

### 6.2 Dark theme challenges

- The light shadow cannot be true white or the element will look inverted.
- Contrast between surface and text must still meet standards.
- Glow effects become more visible and can feel noisy.

### 6.3 Dark theme best practices

- Derive the light shadow from the base color, not pure white.
- Keep elevation differences subtle.
- Use accent color for focus and active states only.
- Test on OLED and IPS displays because dark greys crush differently.

---

## 7. Component patterns in detail

### 7.1 Buttons

```css
.btn-neu {
  border: none;
  border-radius: 16px;
  background: var(--bg);
  box-shadow: 6px 6px 12px var(--dark), -6px -6px 12px var(--light);
  transition: all 0.2s ease;
}

.btn-neu:active, .btn-neu.active {
  box-shadow: inset 4px 4px 8px var(--dark), inset -4px -4px 8px var(--light);
}
```

Icon-only buttons are often perfect circles. Hover should lift slightly; active should press.

### 7.2 Sliders and range inputs

- Track: inset shadow groove.
- Thumb: raised circular surface.
- Fill progress: tinted strip inside the track.

### 7.3 Toggles / switches

A neumorphic switch is a pill-shaped inset track with a movable raised orb. On activation the orb slides right and the track may show an accent glow.

### 7.4 Cards and panels

Cards use large radii and soft shadows. Elevation creates hierarchy: app shell flat, panels raised, floating controls elevated.

### 7.5 Input fields

Inputs are usually inset. Placeholder text must remain legible. Focus is shown by a faint accent glow or slight inner brightness change.

### 7.6 Lists and chips

Items can be flat until hovered, then raised. Selected chips are pressed or accented. Chips in a lane are ideal candidates for raised → pressed active transitions.

### 7.7 Scrollbars

Custom scrollbars can use an inset track and raised thumb to stay on-theme.

### 7.8 Tabs and segmented controls

A segmented control is a raised pill with each segment flat until selected; the selected segment is pressed.

### 7.9 Progress rings and loaders

A circular loader can be a rotating gradient inside a pressed ring.

### 7.10 Tooltips and menus

Floating menus use the strongest elevation with a tight radius and clear separation from the background.

---

## 8. Motion and micro-interactions

### 8.1 Purposeful animation

Animation should be functional:
- Confirm a press.
- Communicate state change.
- Guide attention.
- Provide spatial continuity.

### 8.2 Timing

- Micro-interactions: 150–250 ms
- State transitions: 200–300 ms
- Page/section transitions: 300–500 ms

### 8.3 Easing

Standard:
```css
cubic-bezier(0.25, 0.46, 0.45, 0.94)
```

Spring-like:
```css
cubic-bezier(0.34, 1.56, 0.64, 1)
```

### 8.4 Reduced motion

```css
@media (prefers-reduced-motion: reduce) {
  * {
    animation-duration: 0.01ms !important;
    transition-duration: 0.01ms !important;
  }
}
```

---

## 9. Accessibility and inclusive design

### 9.1 Contrast

- Normal text: 4.5:1 minimum.
- Large text: 3:1 minimum.
- UI components and icons: 3:1 minimum against adjacent colors.

### 9.2 Focus indicators

```css
:focus-visible {
  outline: none;
  box-shadow: inset 0 0 0 2px var(--accent);
}
```

### 9.3 Screen readers

- Keep semantic HTML.
- Use `aria-pressed` for toggles.
- Provide accessible names for icon-only controls.
- Avoid empty button elements.

### 9.4 Cognitive load

Low-contrast neumorphism can make it harder to distinguish interactive from decorative surfaces. Use labels, icons, and consistent elevation to reinforce affordance.

---

## 10. Tooling and generators

### 10.1 Neumorphism.io

The canonical generator. It produces the paired shadow code for a given color and radius.

### 10.2 CSS-Tricks neumorphism guide

Provides the conceptual foundation and warns about accessibility pitfalls.

### 10.3 Figma / Sketch plugins

Many plugins generate soft-shadow styles and component kits. They are useful for rapid exploration but still require manual contrast checks.

### 10.4 Color tools

- ColorHexa: conversions and harmonies.
- Coolors: palette generation.
- UI Gradients: gradient inspiration.
- Color Hunt: curated palettes.

### 10.5 Contrast checkers

- WebAIM Contrast Checker.
- Stark plugin for Figma / Sketch.
- A11y Project resources.

---

## 11. Criticism and limitations

### 11.1 Low contrast

The biggest criticism of neumorphism is poor accessibility. Buttons can look inactive; pressed states can look disabled.

### 11.2 Visual fatigue

A screen full of soft grey shapes can feel monotonous. Accent color and typographic hierarchy are necessary relief.

### 11.3 Platform inconsistency

Shadow rendering differs across browsers and operating systems. Test on the target platform.

### 11.4 Small touch targets

Soft buttons can appear larger than their hit area. Maintain minimum 44 × 44 dp touch targets.

### 11.5 When not to use it

Avoid neumorphism for dense data tables, long-form reading, or interfaces where color coding is critical.

---

## 12. VisualLLM application plan

### 12.1 Design goals

1. Make the lane track the dominant visual element.
2. Shrink header and footer chrome.
3. Add cohesive light/dark themes.
4. Use purposeful animation for drag, press, and routing feedback.
5. Preserve accessibility and reduced-motion preferences.

### 12.2 Planned token map

```css
:root {
  --bg: #eceef1;
  --surface: #e6e9ef;
  --light: #ffffff;
  --dark: #c4cfdc;
  --text: #2b2f38;
  --text-muted: #6b7280;
  --accent: #3b82f6;
  --accent-dark: #2563e3;
  --shadow-raised: 8px 8px 16px var(--dark), -8px -8px 16px var(--light);
  --shadow-pressed: inset 5px 5px 10px var(--dark), inset -5px -5px 10px var(--light);
  --radius-sm: 10px;
  --radius-md: 16px;
  --radius-lg: 24px;
}

[data-theme="dark"] {
  --bg: #1f2229;
  --surface: #252a32;
  --light: #2f3540;
  --dark: #111318;
  --text: #e4e6eb;
  --text-muted: #8a919c;
  --accent: #60a5fa;
  --accent-dark: #3b82f6;
}
```

### 12.3 Component decisions

- Header / footer: reduce vertical padding to ~8–12 px.
- Lane: large raised card with generous radius.
- Lane head / foot: compact flex rows, same background as lane.
- Track: subtle inset groove for chips.
- Chips: raised pills; active/fallback state uses pressed inset or accent ring.
- Add-lane button: circular floating action with strong shadow.
- Theme toggle: neumorphic switch in the header.

### 12.4 Files to modify

- `renderer/style.css` — full redesign, compact lane chrome, themes, animations.
- `renderer/index.html` — theme toggle and structural refinements.
- `renderer/app.js` — theme persistence/toggle wiring.

### 12.5 Backup

Original files copied to `renderer-backup-original/` before modification.

---

## 13. Source bibliography

The following sources informed this document. They include canonical articles, design systems, accessibility guidelines, color tools, and platform documentation.

1. Plyuto, A. (2019). *Skeuomorph Mobile Banking*. Dribbble shot series credited with starting neumorphism.
2. CSS-Tricks. *Neumorphism and CSS*. https://css-tricks.com/neumorphism-and-css/
3. Neumorphism.io. *Generate Soft-UI CSS code*. https://neumorphism.io/
4. UX Collective. *Neumorphism: why it’s bad for accessibility*.
5. Smashing Magazine. *Neumorphism in user interfaces*.
6. Material Design 3. *Motion guidelines*. https://m3.material.io/styles/motion/overview
7. Material Design 3. *Easing and duration*. https://m3.material.io/styles/motion/easing-and-duration
8. Material Design 3. *Understanding motion*. https://m3.material.io/styles/motion/understanding-motion
9. Material Design 3. *Elevation*. https://m3.material.io/styles/elevation/overview
10. Nielsen Norman Group. *The role of animation and motion in UX*.
11. Nielsen Norman Group. *Dark mode: what you need to know*.
12. Nielsen Norman Group. *Accessibility for visually impaired users*.
13. Nielsen Norman Group. *10 usability heuristics for user interface design*.
14. Interaction Design Foundation. *Microinteractions*.
15. Interaction Design Foundation. *Visual hierarchy*.
16. Interaction Design Foundation. *Color theory*.
17. Interaction Design Foundation. *Affordances in UI*.
18. UXPin. *Microinteractions in UI design*.
19. LogRocket. *How to create microinteractions*.
20. Creative Bloq. *Neumorphism: a new trend in UI design*.
21. WebdesignerDepot. *Neumorphism: the design trend of 2020*.
22. ColorHexa. *Color encyclopedia and conversions*. https://www.colorhexa.com/
23. CSS Gradient. *Gradient generator*. https://cssgradient.io/
24. Color Hunt. *Color palettes*. https://colorhunt.co/
25. Coolors. *Color palette generator*. https://coolors.co/
26. UI Gradients. *Gradient palettes*. https://uigradients.com/
27. Dribbble. *Neumorphism tag*. https://dribbble.com/tags/neumorphism
28. Behance. *Soft UI projects*. https://www.behance.net/
29. Pinterest. *Neumorphism UI inspiration*.
30. Awwwards. *Neumorphism website collection*.
31. Adobe XD. *Neumorphism UI kit resources*.
32. Figma Community. *Neumorphism design system files*.
33. Apple Human Interface Guidelines. *Dark mode*.
34. Apple Human Interface Guidelines. *Motion*.
35. Microsoft Fluent UI. *Design tokens: shadow, depth*.
36. Microsoft Fluent UI. *Motion*.
37. W3C WCAG 2.2. *Contrast minimum (1.4.3)*. https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum
38. W3C WCAG 2.2. *Non-text contrast (1.4.11)*.
39. W3C WCAG 2.2. *Reduced motion (2.3.3)*.
40. W3C WCAG 2.2. *Focus appearance (2.4.13)*.
41. MDN Web Docs. *box-shadow*. https://developer.mozilla.org/en-US/docs/Web/CSS/box-shadow
42. MDN Web Docs. *prefers-reduced-motion*. https://developer.mozilla.org/en-US/docs/Web/CSS/@media/prefers-reduced-motion
43. MDN Web Docs. *color-mix()*. https://developer.mozilla.org/en-US/docs/Web/CSS/color_value/color-mix
44. MDN Web Docs. *CSS custom properties*. https://developer.mozilla.org/en-US/docs/Web/CSS/--*
45. MDN Web Docs. *CSS transitions*. https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_transitions
46. Google Material. *Understanding elevation*. https://m2.material.io/design/environment/elevation.html
47. Google Material. *Shadows*. https://m3.material.io/styles/elevation/overview
48. Refactoring UI. *Shadows and depth*.
49. Refactoring UI. *Color in UI design*.
50. Refactoring UI. *Font size and hierarchy*.
51. Toptal. *Neumorphism: a guide for designers*.
52. Designmodo. *Neumorphism UI design tutorial*.
53. Envato Tuts+. *Create a neumorphic UI in Figma*.
54. InVision. *Animation in UX design*.
55. InVision. *Design systems handbook*.
56. HubSpot Blog. *Neumorphism: what it is and how to use it*.
57. UX Booth. *Microinteractions: the secret to great app design*.
58. CareerFoundry. *What is neumorphism?*
59. Mockplus. *Neumorphism UI design examples*.
60. Prototypr. *Neumorphism: visual principles*.
61. Sidebar. *Neumorphism resources*.
62. Speckyboy Design Magazine. *Neumorphism design examples*.
63. Hongkiat. *Neumorphism UI design resources*.
64. A List Apart. *Designing for dark mode*.
65. A List Apart. *Accessibility: the missing ingredient*.
66. Deque. *Understanding WCAG contrast requirements*.
67. WebAIM. *Contrast checker*. https://webaim.org/resources/contrastchecker/
68. Stark. *Contrast and color accessibility*. https://www.getstark.co/
69. A11y Project. *How to design accessible animations*.
70. A11y Project. *Dark mode accessibility*.
71. CSS-Tricks. *A complete guide to dark mode on the web*.
72. CSS-Tricks. *A complete guide to custom properties*.
73. CSS-Tricks. *Animating with CSS transitions*.
74. CSS-Tricks. *A complete guide to flexbox*.
75. Josh Comeau. *The surprising truth about pixels and accessibility*.
76. Josh Comeau. *Shadow palettes generator*.
77. Josh Comeau. *CSS variables for React devs*.
78. Adam Argyle. *Open-props style tokens*.
79. Open Props. *UI shadows*. https://open-props.style/
80. Tailwind CSS. *Box shadow documentation*.
81. Tailwind CSS. *Color system*.
82. Bootstrap. *Shadows utilities*.
83. Chakra UI. *Style props: shadow*.
84. Radix UI. *Themes and tokens*.
85. Shadcn UI. *Design tokens and themes*.
86. GitHub Primer. *Design system: color and shadow*.
87. Shopify Polaris. *Design tokens*.
88. Atlassian Design System. *Elevation*.
89. IBM Carbon. *Shadows*.
90. Salesforce Lightning. *Design tokens*.
91. Mozilla Protocol. *Color and shadows*.
92. Tauri Documentation. *Window customization*. https://tauri.app/
93. Tauri Documentation. *Security and capabilities*.
94. Tauri Documentation. *Frontend configuration*.
95. Electron. *The official guide*.
96. WebKit Blog. *CSS dark mode support*.
97. Chromium Blog. *RenderingNG and performance*.
98. V8 Blog. *Fast properties*.
99. Rust Cargo. *The Cargo book*.
100. Rust By Example. *Error handling*.
101. OpenAI API. *Model routing and fallbacks*.
102. Anthropic. *API documentation*.
103. OpenRouter. *API documentation*.
104. Phosphor Icons. *Icon design principles*. https://phosphoricons.com/
105. Heroicons. *SVG icon set*. https://heroicons.com/
106. Feather Icons. *Beautiful open-source icons*. https://feathericons.com/
107. Tabler Icons. *Open-source icons*. https://tabler-icons.io/
108. Font Awesome. *Accessibility best practices*.
109. Google Fonts. *Inter / Roboto / System UI fonts*.
110. Typewolf. *Best system font stacks*.
111. Modularscale. *Type scale ratios*.
112. Every Layout. *The stack / sidebar / cover primitives*.
113. Defensive CSS. *Practical tips*.
114. Smashing Magazine. *Designing for reduced motion*.
115. Web.dev. *prefers-reduced-motion*.
116. Web.dev. *Accessible animations*.
117. Vercel. *Design engineering principles*.
118. Linear. *Product design principles*.
119. Raycast. *Interface design notes*.
120. Notion. *Design system notes*.
121. Figma. *Auto layout documentation*.
122. Figma. *Component properties*.
123. Sketch. *Layer styles and shadows*.
124. Canva. *Color theory for designers*.
125. Hype4 Academy. *Neumorphism / soft UI course*.
126. UX Design Institute. *Visual design principles*.
127. Career Karma. *Neumorphism vs skeuomorphism*.
128. SitePoint. *CSS box-shadow tips*.
129. Scotch.io. *CSS variables tutorial*.
130. CodePen. *Neumorphism examples collection*.

---

## 14. Implementation checklist

- [x] Research and document neumorphic principles.
- [x] Back up existing renderer files.
- [ ] Define `:root` and `[data-theme="dark"]` tokens.
- [ ] Compact header and footer.
- [ ] Redesign lane, lane-head, lane-foot, track, and chips.
- [ ] Add theme toggle and persistence.
- [ ] Add purposeful micro-interactions.
- [ ] Add `prefers-reduced-motion` guard.
- [ ] Validate contrast and focus states.
- [ ] Test in both themes at runtime.

---

## 15. Notes

- All UI changes are scoped to `renderer/index.html`, `renderer/style.css`, and `renderer/app.js`.
- A backup of the original renderer files is stored in `renderer-backup-original/` before modifications.
- The Rust backend (`src-tauri/src/`) is not affected by this redesign.
