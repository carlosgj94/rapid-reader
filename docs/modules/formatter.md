# Formatter And Content Pipeline

## Purpose

The formatter module is the boundary between raw or normalized content and the structures the reader
can actually present.

It should be open to multiple input sources over time, but it should produce one canonical reading
pipeline.

## Primary Input Model

The primary v1 input is a normalized article package from the backend.

That package should be cleaner and more structured than raw HTML, but it should still leave final
reading preparation to the device.

The backend should not send a fully baked RSVP stream as the dominant model.

## Canonical Content Model

The architecture should revolve around a skeleton `ArticleDocument` concept.

At this phase, that concept should include:

- stable identity
- source classification
- revision identity
- article metadata
- structured body content
- optional assets or asset references
- optional reading hints

This is intentionally a skeleton, not a field-complete schema.

## Structured Body Content

The canonical body representation should support article-oriented blocks such as:

- heading
- paragraph
- quote
- list
- code or preformatted block
- divider or callout
- media placeholder where relevant

The key idea is that source-specific parsing happens once, and the rest of the system sees one
shared content model.

## Formatter Output

The formatter should produce a `ReadingDocument` or equivalent structure that contains:

- RSVP-ready reading units
- stable anchors for resume and navigation
- derived reader metadata
- warnings or degradation markers if formatting had to fall back

The reader should consume this output, not reinterpret the original article package.

## Pipeline Stages

The target pipeline is:

1. ingest source package or adapter output
2. normalize text and structural content
3. segment content into reading-oriented units
4. attach anchors and navigation metadata
5. cache or persist formatter output where useful

Each stage should be testable in isolation.

## Source Adapters

The formatter pipeline should support a `SourceAdapter` concept for future imports such as:

- backend normalized article packages
- EPUB
- TXT

These adapters should converge into the same `ArticleDocument` concept before reader formatting.

## Extension Strategy

EPUB and TXT are extension points, not equal primary inputs in the first implementation pass.

That means:

- the canonical content model should be broad enough to support them later
- the module boundaries should anticipate adapters now
- the initial product architecture should still optimize for synced web articles

## Failure Behavior

If formatter preparation fails:

- the article remains part of the local content set
- the queue may still show the item
- the system should report that the content could not be prepared
- failure should not poison unrelated cached content

## Module Boundaries

The formatter owns:

- normalization
- segmentation
- reading-unit generation
- source adapter contracts

The formatter does not own:

- network fetch
- storage placement policy
- reader UI state
- physical rendering

