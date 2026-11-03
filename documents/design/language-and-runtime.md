# Language and Runtime

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

## Language

Rust.

## Constraints this implies

- **Self-contained.** The server should not require external components (e.g., no separate vector database process to install). See [Components](components.md).
- **Offline-capable.** It should be possible to use Scatterhoard without a network connection.
- **Filesystem-backed, not purely in-memory.** Running an open-weight LLM locally can already be memory-intensive, so primary storage should live on the filesystem, with in-memory caching as an optimization. This also makes Scatterhoard usable as a personal, single-user memory system.
