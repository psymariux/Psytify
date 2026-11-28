# Psytify visual style

**This direction is retired (22 November 2026).** Three rounded panels on black, cover-tinted,
lime as the brand mark and vector icons no longer exist.

The source of truth is in `docs/design/`:

- `PSYTIFY_ART_DIRECTION.md`: vision, screens, where the lime appears («Lime by screen»), Nastro.
- `PSYTIFY_DESIGN_TOKENS.md`: palette, spacing, type, layout, motion.
- `PSYTIFY_COMPONENT_GUIDELINES.md`: components.
- `PSYTIFY_PIXEL_ART_GUIDELINES.md`: sprites and the rendering pipeline.

The values live in the `psytify-design` crate (`design/`); `desktop/src/theme.rs` converts them into GPUI
types and the rest of `ui/` uses only those. In short: monochrome, flush regions divided by a 1 AP line,
no radius, no gradient or cover tint, lime only as a state (playing, selected navigation, switches on, live),
the heart of liked tracks white, pixel icons enlarged on the CPU by an integer factor.
ingrandite su CPU di un fattore intero.
