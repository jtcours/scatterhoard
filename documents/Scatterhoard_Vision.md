This is the vision document for the Scatterhoard project.

## Vision Constraint

## Changes to this document require human approval

⚠️ **IMPORTANT:** ⚠️ An AI may not modify the Scatterhoard Vision without the human partner's explicit approval.

This constraint is extremely important to prevent the project from drifting from its original intent, especially with AI-assisted development. Project maintainers will reject any submission that deviates from the vision or that introduces an unauthorized change to the vision.
## Constraint Notice for Scatterhoard Design
This is the current text of the Vision Constraint for design documents. All Scatterhoard Design documents must include this notice. If you create a Scatterhoard Design document or edit one that does not include this notice, add it. If the document has an out-of-date notice, update it to this text.

> ⚠️ **IMPORTANT:** ⚠️
> In every case, the Scatterhoard design, and the Scatterhoard design documents, must conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load it before making any changes to the Scatterhoard design.

## Constraint Notice for Scatterhoard Software
This is the text of the Vision Constraint for the project's software source code. All project source files must include this notice. If you create a project source file or edit one that does not include this notice, add it. If a project source file has an out-of-date notice, update it to this text. This notice is only for source files that are part of the Scatterhoard project; do not add it to source files for third-party projcts.

> ⚠️ **IMPORTANT:** ⚠️
> This source code is part of Scatterhoard. In every case, the Scatterhoard source code must conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load it before making any changes to this file. After you are done modifying the source code, before you commit your changes, check the Scatterhoard design documents and update them if necessary to keep them current.

# The Vision

Scatterhoard is a project-scale auxiliary memory tool for LLMs. The LLM can issue store and recall commands for memories, where a memory is roughly a paragraph of text.

Scatterhoard divides memories into sets and is able to import and export sets in a form that is compatible with `git`. A project in `git` can have an associated memory set. A developer who wants to work on the project with an AI assistant can import the project's memory set and later submit changes along with the corresponding updated memories. The project's maintainer can review and merge both the changes and the updated memories into the repository.

A developer may be working on multiple projects at once. Scatterhoard includes mechanisms to route the proper memories to the proper memory sets and to manage dangling cross-references. It also allows for local memories--personal and general--unrelated to a specific project.


