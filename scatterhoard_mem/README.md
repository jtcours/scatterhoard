# scatterhoard_mem

This directory holds memory sets for [Scatterhoard](../SETUP.md), the memory system for coding
agents. Each `.json` file is one set, written by Scatterhoard's `export`; edit them through Scatterhoard
rather than by hand.

- **`scatterhoard.json`** is this project's own memory: how Scatterhoard works, how it came to be as it
  is, and how the work on it is done. It is for people working on Scatterhoard.
- **`scatterhoard-cookbook.json`** holds recipes for people *using* Scatterhoard: procedures for
  setting up, moving data, and recovering from mistakes, each run end to end before it was written.
  A session finds the one it needs by recalling what it is trying to do.

## Taking on the cookbook

Take it on read-only, so your sessions can recall its recipes without writing to it. In a session with
Scatterhoard attached:

1. `hoard_sets` with action `create`, name `scatterhoard-cookbook`, and mode `read-only`.
2. `hoard_sets` with action `import`, scope `shared:scatterhoard-cookbook`, and either
   - `file`: the whole path of a downloaded `scatterhoard-cookbook.json`, or
   - `directory`: the root of a clone of this repository. The directory holding both files is fine:
     Scatterhoard reads the one whose header names the set.

`hoard_sets` with action `status` then lists `shared:scatterhoard-cookbook` as `read-only`.
