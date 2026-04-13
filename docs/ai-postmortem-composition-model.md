# AI Usage Postmortem: Composition Model Discussion

A record of how an AI-assisted design conversation about the composition model went wrong twice in similar ways, what the recurring error was, and what the AI should have done instead.

## Context

The conversation was about choosing between a bounds-based composition model and a fixed-cell grid for the asset creator's shape format. The author had already built a bounds-based format around three primitives (Box, Wedge, Corner) with integer coordinates and per-axis stretching. The AI was asked to compare the two models, then to help document the decision, then to walk through T-junctions as the main known limitation.

The conversation produced a useful design document (`composition-model.md`) and a working resolution to the T-junction problem. But it took two explicit pushbacks from the author to get there, both correcting the same kind of error.

## The two errors

### Error 1: Watertightness

When comparing bounds-based composition to a fixed-cell grid, the AI claimed the grid had "free watertightness" as a structural advantage and listed it as a reason to consider switching. The author pushed back: every primitive in the bounds-based format is itself watertight, so any composition of them is also watertight as a point set. The AI's argument was wrong in the sense that mattered.

After the pushback, the AI conceded and clarified: what the grid actually gives you for free is **automatic vertex sharing without a merge pass**, and structural prevention of T-junctions. Neither of those is "watertightness." The original framing imported a generic concern from CAD pipelines that did not apply to the specific format under discussion.

### Error 2: T-junction handling

When asked how to handle T-junctions, the AI offered three options:

1. Forbid T-junctions at authoring time via a load-time validator.
2. Subdivide at mesh-generation time by splitting long faces at T-points.
3. Add explicit "stitch" or "filler" primitives to bridge mismatched scales.

The author then asked the obvious question: "Can't we just fuse everything into a single 3D solid?" Once the AI considered fusion, the cheap answer fell out immediately — for a format where primitives meet at shared faces but never overlap, you don't need general CSG. A three-pass meshing step (vertex weld, internal face cancellation, T-junction repair) produces a single watertight integer-exact manifold while staying entirely in integer arithmetic.

This answer was qualitatively better than any of the original three. It was not on the original list. Suggestion #2 contained one third of it (the T-junction repair pass) but framed narrowly as "patch the defect" rather than "fuse the composition."

## The recurring root cause

Both errors collapse to the same mistake: **the AI reasoned about a generic abstraction ("bounds-based composition") instead of the specific format the author actually had**, and consequently missed properties that made the specific format much more capable than the generic version.

The format's distinctive properties — the ones the AI failed to use until pushed — were:

- **Coordinates are exact integers** with sub-unit precision achieved by rescaling the parent unit. There is no floating-point drift, ever.
- **Primitives meet at shared faces but never overlap.** Two primitives whose bounds touch are fine; two that share interior volume are an authoring error.
- **Composites are first-class** and can be reused by reference, transformed by the integer-rational affine group.

Each of these properties unlocks a cheap answer to a problem that looks expensive in the generic case:

- Integer coordinates make vertex coincidence bit-exact, so welding is a `==` check with no tolerance.
- Non-overlapping primitives make face cancellation a coincident-pair lookup with no boolean kernel.
- Integer collinearity tests make T-junction detection exact.

Together, the three properties turn the meshing pipeline from "needs a CSG kernel" into "three small passes in integer arithmetic." The AI knew all three properties — they had been discussed in the same conversation — and still reached for generic CAD-pipeline answers when thinking about fixes.

## What the AI should have done differently

Before answering any question of the form "how do you handle problem X in this format," the AI should have asked itself:

1. **What does this format make possible that a generic version wouldn't?** List the structural properties explicitly before reasoning about solutions.
2. **Which of those properties are relevant to the problem at hand?** Map each property to whether it weakens or eliminates a step that would normally be expensive.
3. **What's the cheapest solution that uses those properties?** Default to the answer that exploits the structure, not the textbook answer that ignores it.

Concretely, in this conversation:

- Before claiming "the grid has free watertightness," the AI should have asked "what does the bounds-based format have that makes watertightness automatic anyway?" The answer (integer-exact vertex coincidence, non-overlapping primitives) would have made the claim collapse before it was written.
- Before listing three T-junction-specific patches, the AI should have asked "given that primitives meet but don't overlap and coordinates are integer, can the whole pipeline be reduced to a fusion step?" The answer is yes, and the fusion step is cheap.

In both cases, one minute of looking at the actual format's properties would have produced a better answer than several turns of generic reasoning followed by author pushback.

## Pattern to watch for

The failure mode is reaching for **what a generic version of this kind of system would do** instead of **what this specific system makes possible**. It shows up as:

- Importing concerns from analogous tools (CAD pipelines, voxel engines, mesh libraries) without checking whether the analogous constraints apply.
- Listing several incremental fixes when one structural insight would obviate them.
- Treating distinctive format properties as background information rather than load-bearing facts that should drive the answer.

The author corrected this twice in one conversation. The corrections were specific enough and the pattern obvious enough that future sessions on this project should explicitly start from the format's distinctive properties when designing or debugging.

## Outcome

Despite the two errors, the conversation produced:

- A design-decision document (`composition-model.md`) that records the bounds-based choice, its advantages and limitations, and the integer-exact fusion approach to T-junctions.
- A clear resolution to the T-junction problem that preserves all the format's exactness properties.
- This postmortem.

The errors were recoverable because the author was willing to push back rather than accept the first answer. A less-engaged author might have ended up with a worse design built around an unnecessarily heavy CSG pipeline.
