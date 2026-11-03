# Components

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the [Scatterhoard Vision](../Scatterhoard_Vision.md). If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

[← Back to index](index.md)

Scatterhoard is an MCP server and a set of prompts that contain hard-coded operating conventions (see [Conventions](conventions.md)).

The goal is to make the server self-contained, to make adoption as easy as possible, and to allay any fears about exporting personal information. It should not require external components (e.g., the developer should not have to install a separate vector database). Installation should be as simple as downloading and installing the MCP server and maybe adding a boot/launching prompt for the LLM (see [Trigger and Loading](trigger-and-loading.md)). It should be possible to use Scatterhoard without a network connection.

## Internal libraries

The MCP server consists of a set of libraries, each of which could be used independently, and some of which may be third-party components with a compatible license.

- Vector database
- Vector embedding
- Memory graph management system
- MCP services
- Memory import/export
- Other components as identified

## Storage

Running an open-weight LLM locally can be memory-intensive, so the vector database should not be purely in-memory. In-memory caching is fine, but primary storage should use the filesystem. That will also let someone use Scatterhoard as a personal memory system if they like.
