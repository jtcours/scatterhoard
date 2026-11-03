"""Recorded mutations, consumed by mutate.py.

Each entry breaks something deliberately and records whether the test suite is supposed to
notice. `expect = "caught"` means some test must fail, or the tests must hang. `expect =
"survives"` means the tests are *not* expected to notice, and requires a `reason` -- a surviving
mutation has to be explained rather than tolerated, or it is just an untested guard wearing a
note.

`expect = "compiler"` is the third and rarest: the mutation does not compile, and that is the
answer -- the type system is the guard being proved. **It must be asked for by name.** A
mutation stopped by the compiler has not been noticed by any test, so counting it as caught
inflates the corpus with coverage the suite does not have, and in a summary it is
indistinguishable from a mutation a test caught by name. Seven of 243 were found this way on
2026-08-26 and every one was a broken record rather than a deliberate one: replacement text
naming a function that never existed, and replacements that stopped compiling once a struct they
construct grew a field. `--stale` cannot see this class at all -- it checks that `find` still
matches, which for all seven it did; what had rotted was `replace`, which nothing looks at until
it is applied.

`find` is matched literally, not as a regex. Escaping regexes by hand is what made the earlier
ad-hoc version unreliable. `tests` narrows which integration test binaries run, for speed.
"""

MUTATIONS = [
    {
        # The export gate's wording is behaviour, not decoration: the previous phrasing led a session
        # to treat supersession as an expensive fallback and to file a design question against a cost
        # nothing in the project claims. Nothing else pins tone, so without this the guard could be
        # deleted and every test would still pass.
        'name': "Export gate: go back to refusing rather than routing",
        'file': "src/mcp.rs",
        'find': "\"memory {} has entered service -- a second durable copy of it exists -- so the \\",
        'replace': "\"memory {} can no longer be corrected -- a second durable copy of it exists -- so the \\",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The default must keep scanning superseded bodies: a chain folds into one candidate
        # carrying the best evidence any member matched, so a query matching an old claim's wording
        # returns the correction. Flipping the default silently would score every chain on its
        # current text only, which is a real behaviour change and one no other test would notice.
        'name': "Stages default: stop scanning superseded memories",
        'file': "src/store.rs",
        'find': "            include_superseded: true,",
        'replace': "            include_superseded: false,",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # The shipping default is `Calibrated` -- each store's own level -- and `None` is the
        # ablation baseline. The two are indistinguishable on a store that has never calibrated
        # anything, which is every store in `tests/recall.rs`, so this ran green against the wrong
        # binaries for one pass: the tests that can tell them apart are the ones in `tests/floor.rs`
        # that set a floor first. A default silently demoted to `None` would disable the whole
        # mechanism for every shipping caller while every measurement still read as before.
        'name': "Stages default: ship with a floor",
        'file': "src/store.rs",
        'find': "            include_superseded: true,\n            floor: Floor::Calibrated,",
        'replace': "            include_superseded: true,\n            floor: Floor::None,",
        'tests': ["recall", "floor"],
        'expect': "caught",
    },
    {
        'name': "Floor: keep everything regardless",
        'file': "src/store.rs",
        'find': "        retain_by(&mut hits, &keep);",
        'replace': "        let _ = &keep;",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # `Stages::default()` is the only thing making an ablation comparable to the product: every
        # measurement of what a scoring decision contributes is taken against it as the baseline.
        # Three of its four fields are pinned here, one per decision, because a default that quietly
        # stopped matching the shipping scorer would make every such measurement wrong while every
        # test still passed -- which is the failure that produced this seam in the first place.
        # Inverted 2026-10-02, when the cut came out of what ships: it was `stop cutting`.
        'name': "Stages default: cut at the largest gap again",
        'file': "src/store.rs",
        'find': "            gap_cut: false,\n            include_superseded: true,",
        'replace': "            gap_cut: true,\n            include_superseded: true,",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        'name': "Stages default: centre per store instead of combined",
        'file': "src/store.rs",
        'find': "            centring: Centring::Combined,\n            walk: Walk::measured(),",
        'replace': "            centring: Centring::PerStore,\n            walk: Walk::measured(),",
        'tests': ["recall", "cross_store"],
        'expect': "caught",
    },
    {
        'name': "Stages default: no walk at all",
        'file': "src/store.rs",
        'find': "            walk: Walk::measured(),\n            inherited_weight:",
        'replace': "            walk: Walk::none(),\n            inherited_weight:",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # `incoming-edges-invisible-on-the-wire` was the reverse index being real, tested, and
        # unreachable through the only interface that exists. The two mutations below pin the two
        # ways it could come back: the field silently emptying, and the label filter returning --
        # excluding `supersedes` because `superseded_by` already reports it is exactly the
        # hard-coded slice that was the defect.
        'name': "Incoming edges: report none",
        'file': "src/store.rs",
        'find': "        Ok(self\n            .incoming(id)?\n            .into_iter()\n            .map(|(source, label)| IncomingEdge { source, label })\n            .collect())",
        'replace': "        let _ = id;\n        Ok(Vec::new())",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "Incoming edges: hide supersession behind superseded_by",
        'file': "src/store.rs",
        'find': "            .map(|(source, label)| IncomingEdge { source, label })",
        'replace': "            .filter(|(_, label)| *label != LinkLabel::Supersedes)\n            .map(|(source, label)| IncomingEdge { source, label })",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "Confidence: drop the non-finite guard",
        'file': "src/memory/confidence.rs",
        'find': "if !value.is_finite() {",
        'replace': "if false {",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # Anchored on the `NotASingleWord` value, which only `Keyword::new` has. `MemoryText::new`
        # ends with an identical `Ok(Self(trimmed.to_owned()))` after an identical `});\n        }`,
        # so the previous anchor was still ambiguous and the runner mutated the wrong constructor --
        # caught, but by a `MemoryText` test, which would have read as proof of a guard that was
        # never exercised. Re-anchored 2026-08-18, the second time this entry has needed it.
        'name': "Keyword: fold the stored value again",
        'file': "src/memory/text.rs",
        'find': "                value: trimmed.to_owned(),\n            });\n        }\n\n        Ok(Self(trimmed.to_owned()))",
        'replace': "                value: trimmed.to_owned(),\n            });\n        }\n\n        Ok(Self(trimmed.to_lowercase()))",
        'tests': ["memory_model", "store"],
        'expect': "caught",
    },
    {
        'name': "Keyword: stop folding the lookup key",
        'file': "src/memory/text.rs",
        'find': "    let normalised: String = text.nfkc().collect();\n    caseless::default_case_fold_str(&normalised)\n        .nfkc()\n        .collect()",
        'replace': "    text.to_owned()",
        'tests': ["memory_model", "store"],
        'expect': "caught",
    },
    {
        'name': "store: search the stored spelling instead of the folded key",
        'file': "src/store.rs",
        'find': "             WHERE folded IN ({placeholders})",
        'replace': "             WHERE keyword IN ({placeholders})",
        'tests': ["store", "mcp"],
        'expect': "caught",
    },
    {
        'name': "store: skip the backfill when adding the folded column",
        'file': "src/store.rs",
        'find': '            self.connection\n                .execute("UPDATE keywords SET folded = keyword", [])?;',
        'replace': "",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "SetName: forbid dots again",
        'file': "src/memory/scope.rs",
        'find': "|| *c == '_' || *c == '.'",
        'replace': "|| *c == '_'",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        'name': "SetName: allow punctuation at the edges",
        'file': "src/memory/scope.rs",
        'find': "if !character.is_ascii_alphanumeric() {",
        'replace': "if false {",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        'name': "LinkLabel: rename the serialised form",
        'file': "src/memory/link.rs",
        'find': '            Self::RelatedTo => "related_to",',
        'replace': '            Self::RelatedTo => "relatedTo",',
        'tests': ["memory_model", "memory_file"],
        'expect': "caught",
    },
    # --- memory file ------------------------------------------------------------
    {
        'name': "file: allow duplicate memory ids",
        'file': "src/memory_file.rs",
        # Re-pointed 2026-08-07: the duplicate check moved from memories to records when tombstones
        # started travelling in the file, so `memory.id` became `record.id()` and this mutation
        # silently stopped applying. Caught by the runner's own NOT APPLIED, not by a red test.
        'find': "        if let Some(first) = seen.insert(record.id(), number) {",
        'replace': "        if let Some(first) = seen.insert(record.id(), number).filter(|_| false) {",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        'name': "file: skip the declared-scope check",
        'file': "src/memory_file.rs",
        'find': "    check_scope(expected, &header.scope)?;",
        'replace': "",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        'name': "file: let AnyShared accept a local file",
        'file': "src/memory_file.rs",
        'find': "Scope::Local => false,",
        'replace': "Scope::Local => true,",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    # --- store ------------------------------------------------------------------
    {
        # Moved from src/store.rs to src/memory/scope.rs on 2026-08-11, when the tool surface began
        # publishing this same spelling and a private copy in the store would have restored the
        # two-writers defect across modules. Re-anchored rather than retired: the property it
        # guards -- one place spells the prefix -- is unchanged, only its address moved.
        'name': "scope: change the shared prefix in its one place",
        'file': "src/memory/scope.rs",
        'find': 'const SHARED_PREFIX: &str = "shared:";',
        'replace': 'const SHARED_PREFIX: &str = "project:";',
        'tests': ["store", "memory_model"],
        'expect': "caught",
    },
    {
        # The decoder is now `Scope::from_key`. Same defect being guarded: a second hand-written
        # copy of the prefix, which is what drifted in 2026-08-04 when the writer was renamed and
        # the reader kept matching a prefix nothing produced.
        'name': "scope: spell the shared prefix separately in the decoder",
        'file': "src/memory/scope.rs",
        'find': "        match key.strip_prefix(SHARED_PREFIX) {",
        'replace': '        match key.strip_prefix("project:") {',
        'tests': ["store", "memory_model"],
        'expect': "caught",
    },
    {
        'name': "store: skip the write-time cycle check",
        'file': "src/store.rs",
        'find': "        self.refuse_session_writes_if_read_only()?;\n        self.check_no_cycle(memory)?;\n        self.check_control_marks_known(memory)?;",
        'replace': "        self.refuse_session_writes_if_read_only()?;\n        self.check_control_marks_known(memory)?;",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "store: never report a supersession tip",
        'file': "src/store.rs",
        'find': "            if replacements.is_empty() && current != id {",
        'replace': "            if false {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "store: treat a cross-set supersession as resolved",
        'file': "src/store.rs",
        'find': "                        LinkTarget::SameSet(target) => Some(*target),\n                        LinkTarget::CrossSet { .. } => None,",
        'replace': "                        LinkTarget::SameSet(target) => Some(*target),\n                        LinkTarget::CrossSet { .. } => Some(id),",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # **The detector's verdict, not its existence.** There are two cycle defences and neither is
        # observable alone: the `seen` set catches a revisit and the depth limit is a backstop, so
        # removing either leaves the other reporting `Cycle` and no test can tell. Removing *both*
        # is what hangs -- and a hang is a real answer the runner classifies as caught.
        #
        # **That is not what this mutation does, and the reason is worth recording.** The first
        # repair on 2026-08-26 did exactly that: one contiguous edit disabling the `seen` return and
        # resetting the depth counter, producing a genuine non-terminating pass. It found something
        # better than it was looking for -- the runner's timeout could not fire, because `cargo test`
        # launches the test binaries and killing cargo left one spinning with the output pipe open.
        # The `timeout` outcome was unreachable, and the four-hour wedged pass recorded in the
        # `coding-lessons` set on 2026-08-18 was that same bug read as a problem with test helpers.
        # `run` now kills the process group, and `mutate.py --self-check` asks that question
        # directly, in three seconds, of the script it actually concerns.
        #
        # So this mutation went back to what a *test* can see: the revisit is detected and answers
        # something other than `Cycle`. Two seconds instead of five minutes, caught by name, and the
        # timeout path is exercised where it lives rather than by an expensive proxy.
        #
        # **Retargeted 2026-09-14**, when the revisit stopped being the verdict. A revisit is a diamond
        # as often as a loop -- `a-healed-fork-reports-its-ancestor-as-a-cycle` -- so the walk now only
        # avoids walking a memory twice, and `closes_a_loop` decides after it. The mutation follows
        # the verdict to its new line; the one below guards the distinction that moved it.
        'name': "store: answer a supersession loop with anything but a cycle",
        'file': "src/store.rs",
        'find': """        if Self::closes_a_loop(&seen, &edges) {
            return Ok(Walked::Cycle);
        }""",
        'replace': """        if Self::closes_a_loop(&seen, &edges) {
            return Ok(Walked::Current);
        }""",
        'tests': ["store", "sets"],
        'expect': "caught",
    },
    {
        # The defect the retargeting fixed, put back: a memory reached a second time is called a loop.
        # A healed fork reaches its healing memory once from each branch, so this reports the shared
        # ancestor -- the id every older link points at -- as corrupt.
        'name': "store: treat a memory reached twice as a supersession loop",
        'file': "src/store.rs",
        'find': """                if seen.insert(replacement) {
                    frontier.push(replacement);
                }""",
        'replace': """                if !seen.insert(replacement) {
                    return Ok(Resolution::Cycle);
                }
                frontier.push(replacement);""",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The backstop, and it is genuinely subsumed while the `seen` set holds -- traced rather
        # than assumed. `seen` bounds the walk to the number of distinct memories, so the depth limit
        # can only fire on a legitimate chain longer than it, where what it produces is a *false*
        # `Cycle`. It is a cap on pathological input rather than the cycle defence its name suggests,
        # so no test can distinguish it and none should be contrived to.
        #
        # The line stays, because it states the rule and is what holds if `seen` is ever narrowed --
        # the same standard applied to `neighbourhood: let a store resolve an id it does not hold`.
        'name': "store: remove the supersession depth limit",
        'file': "src/store.rs",
        'find': """            if depth > MAX_SUPERSESSION_DEPTH {
                return Ok(Walked::Cycle);
            }""",
        'replace': "",
        'tests': ["store"],
        'expect': "survives",
        'reason': "The `seen` set bounds the walk to the number of distinct memories and "
                  "`closes_a_loop` catches every cycle after it, so this can only fire on a "
                  "legitimate chain longer than the limit -- where it reports a cycle that is not "
                  "one. Subsumed as a defence; kept as a cap.",
    },
    # Expected to be a compile error rather than a hang: removing only the depth guard leaves the
    # `seen` set, and removing only `seen` leaves the depth guard. Confirming they are two independent
    # defences needs both gone, which is exactly what the ad-hoc runs could not express safely.
    {
        'name': "store: drop the reverse index",
        'file': "src/store.rs",
        'find': "             -- second structure that could fall out of step with the edges themselves.\n             CREATE INDEX IF NOT EXISTS links_by_target ON links(target);",
        'replace': "             -- second structure that could fall out of step with the edges themselves.\n             SELECT 1;",
        'tests': ["store"],
        'expect': "survives",
        'reason': """The index is a performance property, not a correctness one. Correctness comes from the
links table itself, which SQLite will happily scan without it. At corpus scale the scan is
indistinguishable from the indexed lookup, so no test can tell — and a test that could would be
measuring speed at 39 memories, which would be measuring nothing.""",
    },
    {
        # **Was `survives` until 2026-08-30, and the change is the useful part.** This statement was
        # redundant with the cascade from `INSERT OR REPLACE`, which deleted the memory row and took
        # its links with it. `forgetting-a-chains-middle-makes-a-stale-conclusion-current-again`
        # removed that foreign key -- a cascade was destroying the ancestry a tombstone now keeps --
        # so the delete is the only thing clearing a rewritten memory's old edges, and dropping it
        # leaves a withdrawn link in the store for ever. The corpus caught the change in meaning: no
        # test failed when the key was dropped, because the behaviour was identical; this mutation
        # flipping from survives to caught is the whole record that the schema moved underneath it.
        'name': "store: stop clearing links before rewriting",
        'file': "src/store.rs",
        'find': '    connection.execute("DELETE FROM links WHERE source = ?1", params![id])?;',
        'replace': "",
        'tests': ["store"],
        'expect': "caught",
    },

    # --- embedding ------------------------------------------------------------
    {
        'name': "embedding: accept a vector with no direction",
        'file': "src/embedding.rs",
        'find': "if components.iter().all(|value| *value == 0.0) {",
        'replace': "if false {",
        'tests': ["embedding"],
        'expect': "caught",
    },
    {
        'name': "embedding: stop removing the centroid",
        'file': "src/embedding.rs",
        'find': "let centroid = self.centroid();\n        self.scan(query, k, centroid.as_ref().map(|c| c.0.as_slice()))",
        'replace': "self.scan(query, k, None)",
        'tests': ["embedding"],
        'expect': "caught",
    },
    {
        'name': "embedding: let the dimension check pass anything",
        'file': "src/embedding.rs",
        'find': "if vector.dimension() != self.dimension {",
        'replace': "if false {",
        'tests': ["embedding"],
        'expect': "caught",
    },
    {
        'name': "embedding: stop subtracting the old vector on replace",
        'file': "src/embedding.rs",
        'find': """        if let Some(previous) = self.vectors.insert(id, vector.0.clone()) {
            for (total, old) in self.sum.iter_mut().zip(&previous) {
                *total -= old;
            }
        }""",
        'replace': "        self.vectors.insert(id, vector.0.clone());",
        'tests': ["embedding"],
        'expect': "caught",
    },
    {
        'name': "embedding: stop subtracting on remove",
        'file': "src/embedding.rs",
        'find': """        if let Some(previous) = self.vectors.remove(&id) {
            for (total, old) in self.sum.iter_mut().zip(&previous) {
                *total -= old;
            }
        }""",
        'replace': "        self.vectors.remove(&id);",
        'tests': ["embedding"],
        'expect': "caught",
    },
    # --- embedder -------------------------------------------------------------
    {
        'name': "embedder: skip the model-presence check",
        'file': "src/embedder.rs",
        # Re-pointed 2026-08-08: the weights path was hoisted into a binding so it could be hashed
        # for the model identity, which moved the text this quoted.
        'find': "if !weights.exists() {",
        'replace': "if false {",
        'tests': ["embedder"],
        'expect': "caught",
    },
    {
        'name': "input: stop including why and keywords",
        'file': "src/embedding.rs",
        # Re-anchored 2026-09-19: the body moved into `embedding_input_of` when the write door
        # needed to measure the same text from the parts.
        'find': """    if let Some(why) = why {
        parts.push(why);
    }
    parts.extend(keywords.iter().map(crate::memory::Keyword::as_str));""",
        'replace': "",
        'tests': ["embedding_input"],
        'expect': "caught",
    },
    {
        'name': "input: start embedding context too",
        'file': "src/embedding.rs",
        # Re-anchored 2026-09-19 with the same refactor. `embedding_input_of` takes parts and
        # never sees `context`, so the place context could wrongly enter is now the caller that
        # holds the whole memory.
        'find': """    embedding_input_of(
        memory.content.as_data(),
        memory.why.as_ref().map(crate::memory::MemoryText::as_data),
        &memory.keywords,
    )""",
        'replace': """    let mut input = embedding_input_of(
        memory.content.as_data(),
        memory.why.as_ref().map(crate::memory::MemoryText::as_data),
        &memory.keywords,
    );
    if let Some(context) = &memory.context {
        input.push(' ');
        input.push_str(context.as_data());
    }
    input""",
        'tests': ["embedding_input"],
        'expect': "caught",
    },
    # --- recall ---------------------------------------------------------------
    {
        # Re-anchored 2026-08-18: `Store::recall` now delegates to `recall_across`, so the early
        # returns it used to carry are gone and this had to move to the one that survives.
        'name': "recall: claim semantic search works with no model",
        'file': "src/store.rs",
        'find': "        return Ok(Recall {\n            hits: Vec::new(),\n            searched: false,",
        'replace': "        return Ok(Recall {\n            hits: Vec::new(),\n            searched: true,",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # Re-pointed 2026-08-19: validity now resolves before the ranking, so these live in the
        # candidate fold rather than in the admission loop.
        'name': "recall: return the superseded memory itself",
        'file': "src/store.rs",
        'find': "            Ranked::Replaced(tip) => (tip, Some(at.id), false, false, false),",
        'replace': "            Ranked::Replaced(_) => (at, None, false, false, false),",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # Re-pointed 2026-08-19: validity now resolves before the ranking, so these live in the
        # candidate fold rather than in the admission loop.
        'name': "recall: stop reporting which memory was matched",
        'file': "src/store.rs",
        'find': "            matched: candidate.matched,",
        'replace': "            matched: None,",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        'name': "recall: stop persisting vectors",
        'file': "src/store.rs",
        # Re-anchored 2026-09-11: the insert moved into `write_embeddings`, which `persist` calls for
        # every path, and now writes a verdict row as well as a vector.
        'find': """        connection.execute(
            "INSERT OR REPLACE INTO vectors (memory_id, vector) VALUES (?1, ?2)",
            params![id.to_string(), blob],
        )?;""",
        'replace': "",
        'tests': ["recall"],
        'expect': "caught",
    },
    # --- mcp ------------------------------------------------------------------
    {
        # **The one mutation the compiler is supposed to stop**, and the reason `expect` has a third
        # value at all. `documents/design/decision-placement.md` says the software cannot judge which
        # set a memory belongs to, so the decision moves to the LLM and *the type system refuses to
        # let it be skipped* -- adding, by name, that "`Scope` deriving `Default` would quietly undo
        # this, so it must not". Asking serde to default the field is exactly that undoing, and it
        # does not compile: `Scope` implements no `Default` for it to reach.
        #
        # A test cannot prove this. There is no call that omits the scope to write a test around --
        # that is the whole point of making it unrepresentable rather than refused.
        #
        # Repaired 2026-08-26: the previous replacement named a function that never existed, so it
        # failed to compile for a reason that had nothing to do with the guard, while reporting as
        # though it had proved it.
        'name': "mcp: make scope optional on memorize",
        'file': "src/mcp.rs",
        'find': "            scope: Scope,\n            content: MemoryText,",
        'replace': "            #[serde(default)]\n            scope: Scope,\n            content: MemoryText,",
        'tests': ["mcp"],
        'expect': "compiler",
    },
    {
        'name': "server: stop embedding stores as they are added",
        'file': "src/mcp.rs",
        'find': "    pub fn add_store(&mut self, mut store: Store) {\n        if let Some(embedder) = &self.embedder {\n            store.set_embedder(Arc::clone(embedder));\n        }",
        'replace': "    pub fn add_store(&mut self, mut store: Store) {\n        if false && let Some(embedder) = &self.embedder {\n            store.set_embedder(Arc::clone(embedder));\n        }",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "server: skip the stores already held when the model arrives",
        'file': "src/mcp.rs",
        'find': "        for store in self.stores.values_mut() {\n            store.set_embedder(Arc::clone(&embedder));\n        }",
        'replace': "",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "memorize: never refuse an unstated keyword set",
        'file': "src/mcp.rs",
        'find': "    if keywords.is_some() {\n        return None;\n    }",
        'replace': "    if true {\n        return None;\n    }",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The guard that matters is not *that* it refuses but *what it keys on*. This mutation
        # leaves the refusal in place and makes it treat an explicitly empty list as if nothing had
        # arrived -- which is the silent-loss half of
        # `a-keyword-cannot-be-retired-once-a-memory-has-been-exported` reintroduced as a refusal
        # instead. Only `superseding_with_an_explicitly_empty_keyword_list_is_accepted` can see it;
        # the refusal test passes either way.
        'name': "memorize: treat an empty keyword list as unstated",
        'file': "src/mcp.rs",
        'find': "    if keywords.is_some() {\n        return None;\n    }",
        'replace': "    if keywords.is_some_and(|k| !k.is_empty()) {\n        return None;\n    }",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "mcp: stop noticing keywords when deciding which search to run",
        'file': "src/mcp.rs",
        'find': "        let (memories, coverage) = match (&params.query, params.keywords.is_empty()) {",
        'replace': "        let (memories, coverage) = match (&params.query, true) {",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "mcp: assume semantic search is available on the keyword path",
        'file': "src/mcp.rs",
        # Re-anchored 2026-08-09: the fold became a loop when `can_search_by_meaning` started
        # reading the model state and so became fallible. The whole computation is the anchor, not
        # its first line — pointing at the initialiser alone left the per-store check intact, so the
        # mutation survived and said only that nothing covers the empty-store case.
        'find': "                    if !store\n                        .can_search_by_meaning()",
        'replace': "                    if false\n                        && !store.can_search_by_meaning()",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "mcp: reply to notifications",
        'file': "src/mcp.rs",
        'find': "        let id = id?;",
        'replace': "        let id = id.unwrap_or(Value::Null);",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "mcp: drop the unavailable-search note",
        'file': "src/mcp.rs",
        'find': "        if !coverage.complete {",
        'replace': "        if false {",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "mcp: create a store for an unknown scope",
        'file': "src/mcp.rs",
        # Re-pointed 2026-08-07: `store_for` was deleted when recall started needing `&mut` to
        # refresh its vector cache, leaving `store_for_mut` as the only scope guard on the surface.
        'find': "            Ok(self.stores.get_mut(scope).expect(\"just checked\"))",
        'replace': "            Ok(self.stores.values_mut().next().expect(\"just checked\"))",
        'tests': ["mcp"],
        'expect': "caught",
    },
    # --- cross-store ----------------------------------------------------------
    {
        'name': "union: centre each store on its own mean",
        'file': "src/store.rs",
        'find': "            Centring::Combined => store.vectors.nearest_centred_on(&vector, width, &centroid),",
        'replace': "            Centring::Combined => store.vectors.nearest(&vector, width),",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        'name': "union: ignore a store that cannot be searched",
        'file': "src/store.rs",
        # Re-anchored 2026-08-09: completeness is now measured against the *searchable* subset,
        # since a store holding another model's vectors is excluded from the scan rather than
        # merely making the answer partial.
        'find': "    let complete = !stores.is_empty() && skipped.is_empty();",
        'replace': "    let complete = !stores.is_empty();",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        'name': "union: weight the centroid by store rather than by memory",
        'file': "src/embedding.rs",
        'find': "        count += len;",
        'replace': "        count += 1;",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    # --- other recall paths ---------------------------------------------------
    {
        # Re-pointed 2026-08-19 when `Found` began carrying the whole `Resolution`, and again
        # 2026-08-20 when the body moved into `found_for` for `crockford-base32-ids`.
        'name': "lookup: substitute a superseded memory",
        'file': "src/store.rs",
        'find': "        let validity = self.resolve_supersession(id)?;\n        let Some(memory) = self.get(id)? else {",
        'replace': "        let id = match self.resolve_supersession(id)? {\n            Resolution::Superseded { ref tips, .. } => *tips.first().unwrap_or(&id),\n            _ => id,\n        };\n        let validity = self.resolve_supersession(id)?;\n        let Some(memory) = self.get(id)? else {",
        'tests': ["other_paths"],
        'expect': "caught",
    },
    {
        'name': "keywords: stop ranking by match count",
        'file': "src/store.rs",
        'find': '             ORDER BY matches DESC, memory_id ASC LIMIT {KEYWORD_BATCH}',
        'replace': '             ORDER BY matches ASC, memory_id ASC LIMIT {KEYWORD_BATCH}',
        'tests': ["other_paths"],
        'expect': "caught",
    },
    {
        'name': "memorize: stop reporting the keyword delta",
        'file': "src/mcp.rs",
        'find': "        report_keyword_deltas(&mut result, &deltas);",
        'replace': "",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The delta is *also* a confirmation, and this is what proves that half is covered. A caller
        # who re-typed the old set faithfully learns so from `dropped: []`; suppressing the report
        # when nothing was dropped would leave them unable to tell a faithful re-typing from a
        # server that did not look.
        'name': "memorize: report the delta only when something was dropped",
        'file': "src/mcp.rs",
        'find': "    if deltas.is_empty() {\n        return;\n    }\n    let dropped_any",
        'replace': "    if deltas.is_empty() || !deltas.iter().any(|d| !d.dropped.is_empty()) {\n        return;\n    }\n    let dropped_any",
        'tests': ["mcp"],
        'expect': "caught",
    },
    # --- the link delta: step 1 of out-links-stay-with-the-record-that-wrote-them (2026-09-24) --
    {
        'name': "memorize: stop reporting the link delta",
        'file': "src/mcp.rs",
        'find': "        report_link_deltas(&mut result, &link_changes);",
        'replace': "",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Like its keyword sibling: `left_behind: []` is the confirmation a faithful re-typing earns,
        # and without it *nothing was left behind* reads the same as *nothing was checked*.
        'name': "memorize: report the link delta only when something was left behind",
        'file': "src/mcp.rs",
        'find': "    if deltas.is_empty() {\n        return;\n    }\n    let left_any",
        'replace': "    if deltas.is_empty() || !deltas.iter().any(|d| !d.left_behind.is_empty()) {\n        return;\n    }\n    let left_any",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The superseded memory's own chain is history, not a claim a replacement could restate.
        'name': "link delta: list the superseded memory's own supersedes link as left behind",
        'file': "src/mcp.rs",
        'find': "!matches!(link.label, LinkLabel::Supersedes | LinkLabel::SupersededBy);",
        'replace': "!matches!(link.label, LinkLabel::SupersededBy);",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # A changed label is a different claim about the same target.
        'name': "link delta: treat a link to the same target under another label as restated",
        'file': "src/mcp.rs",
        'find': ".any(|new| new.label == old.label && new.target.id() == old.target.id())",
        'replace': ".any(|new| new.target.id() == old.target.id())",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Ids are unique across sets; differing `target_keywords` must not make a restatement a drop.
        'name': "link delta: compare whole targets rather than ids",
        'file': "src/mcp.rs",
        'find': ".any(|new| new.label == old.label && new.target.id() == old.target.id())",
        'replace': ".any(|new| new.label == old.label && new.target == old.target)",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Step 3 of out-links-stay-with-the-record-that-wrote-them (2026-09-24). Removing the guard
        # carries a superseded memory's own links onto its tip again, so a link its replacement
        # dropped is still walked.
        'name': "walk: carry a superseded memory's out-links onto its tip",
        'file': "src/store.rs",
        'find': "            if neighbourhood.tip_of(source) != source {\n                continue;\n            }\n",
        'replace': "            let source = neighbourhood.tip_of(source);\n",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # the-calibrate-note-promises-a-floor-that-lowers-itself, fixed 2026-09-24. Puts the removed
        # self-lowering rule back into the note a calibration that set a floor returns.
        'name': "calibrate: promise a floor that lowers itself again",
        'file': "src/mcp.rs",
        'find': "scores under it is still silenced, and counts as evidence at the next \\\n",
        'replace': "scores under it lowers itself on the spot, rather than at the next \\\n",
        'tests': ["sets"],
        'expect': "caught",
    },
    # --- note-when-only-uncalibrated-sets-answer (2026-09-24) ------------------------------------
    {
        # One floored set answering is enough to leave the answer unmarked.
        'name': "unfloored answer: mark when any hit comes from a set without a floor",
        'file': "src/store.rs",
        'find': "        && from.iter().all(|store| floors[*store].is_none())",
        'replace': "        && from.iter().any(|store| floors[*store].is_none())",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # With no floor anywhere nothing declined, so there is nothing for the remaining hits to
        # stand in for -- and a waiver clears every floor, which this condition is what honours.
        'name': "unfloored answer: mark even when no searched set has a floor",
        'file': "src/store.rs",
        'find': "        && floors.iter().any(Option::is_some)\n",
        'replace': "",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "unfloored answer: stop reporting it",
        'file': "src/mcp.rs",
        'find': "        if !coverage.answered_only_without_a_floor.is_empty() {",
        'replace': "        if false {",
        'tests': ["mcp"],
        'expect': "caught",
    },
    # --- calibration-from-self-queries (2026-09-24) ---------------------------------------------
    {
        # Labels outrank the set's own memories; without the guard a label floor is overwritten by
        # an inferred one whenever too few new labels have arrived.
        'name': "self-query floor: replace a floor read from labels",
        'file': "src/store.rs",
        'find': "            if self.floor_source()? != Some(FloorSource::Labels)\n                && let",
        'replace': "            if true\n                && let",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        'name': "self-query floor: count superseded memories as claims",
        'file': "src/store.rs",
        'find': "            if !matches!(self.resolve_supersession(id)?, Resolution::Current) {\n                continue;\n            }\n            let Some(memory) = self.get(id)? else {",
        'replace': "            let Some(memory) = self.get(id)? else {",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        'name': "self-query floor: let a memory answer its own question",
        'file': "src/store.rs",
        'find': "                .filter(|(other, _)| *other != index)\n",
        'replace': "",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        'name': "self-query floor: record no source for a floor read from labels",
        'file': "src/store.rs",
        'find': "        self.record_floor_source(FloorSource::Labels)?;\n",
        'replace': "",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        'name': "self-query floor: leave the source behind when the floor goes",
        'file': "src/store.rs",
        'find': "            \"DELETE FROM meta WHERE key IN ('non_match_floor', 'non_match_floor_source')\",",
        'replace': "            \"DELETE FROM meta WHERE key = 'non_match_floor'\",",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        'name': "self-query floor: offer it to a set too small for it",
        'file': "src/mcp.rs",
        'find': "            if floor.is_none()\n                && labelled < LABELS_PER_CALIBRATION\n                && memories >= LABELS_PER_CALIBRATION\n",
        'replace': "            if floor.is_none()\n                && labelled < LABELS_PER_CALIBRATION\n",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The guard added 2026-09-02. Removing it restores the substitution, so a term the
        # replacement dropped starts matching again -- the behaviour
        # `a-keyword-cannot-be-retired-once-a-memory-has-been-exported` exists to remove. Its
        # sibling below proves resolution still happens at all; this one proves the *result* of the
        # resolution is what decides whether the hit is kept.
        'name': "keywords: reach through supersession again",
        'file': "src/store.rs",
        'find': "            if recalled.matched.is_some() {\n                continue;\n            }",
        'replace': "            if false {\n                continue;\n            }",
        'tests': ["mcp", "other_paths"],
        'expect': "caught",
    },
    {
        'name': "keywords: skip validity resolution",
        'file': "src/store.rs",
        'find': "            let Some(recalled) = self.store.resolved_hit(id, 0.0, 0.0)? else {",
        # Re-anchored 2026-09-29: the loop moved into `KeywordCursor`, which reaches the store
        # through `self.store`. Re-anchored 2026-09-02: the loop became a `let ... else` when keyword search
        # stopped reaching through supersession, so the old `find` matched nothing and the
        # mutation applied to nothing. It still proves the same thing, and proves more than
        # it used to: without resolution a superseded memory is returned as itself, and its
        # keywords match -- which is exactly what this path now exists to prevent.
        # Repaired 2026-08-26: `Recalled` gained `disputed_by`, `in_cycle` and
        # `superseded_beyond_this_set` after this was written, so the replacement stopped compiling
        # and the mutation reported as caught by the compiler rather than by the test named below.
        'replace': '            let Some(recalled) = self.store.get(id)?.map(|memory| Recalled {\n                memory,\n                scope: self.store.scope.clone(),\n                incoming: Vec::new(),\n                disputed_by: Vec::new(),\n                referenced_from: Vec::new(),\n                similarity: 0.0,\n                inherited: 0.0,\n                matched: None,\n                in_cycle: false,\n                superseded_beyond_this_set: false,\n            }) else {',
        'tests': ["other_paths"],
        'expect': "caught",
    },
    # --- the advertised schema ------------------------------------------------
    {
        'name': "schema: advertise recall's optional arguments as required",
        'file': "src/mcp.rs",
        'find': '                "type": "object",\n                "properties": {\n                    "scope": scope_schema(',
        'replace': '                "type": "object",\n                "required": ["scope", "query"],\n                "properties": {\n                    "scope": scope_schema(',
        'tests': ["mcp"],
        'expect': "caught",
    },
    # --- what the server publishes about itself -------------------------------
    {
        # The point of generating the list is that it cannot fall behind the enum. Hand-writing a
        # subset is exactly the drift that used to be undetectable.
        'name': "schema: hand-write the link labels instead of deriving them",
        'file': "src/mcp.rs",
        'find': '    LinkLabel::ALL\n        .iter()\n        .map(|label| format!("`{}`", label.name()))',
        'replace': '    ["supersedes", "related_to"]\n        .iter()\n        .map(|label| format!("`{label}`"))',
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # `scope` published a `oneOf` when this was written and publishes a scalar now, so the
        # anchor moved -- but the property is the same one and matters more than it did: a scope
        # with no declared type is exactly what one client produces by reducing the alternation, and
        # an object argument then arrives as text.
        'name': "schema: describe scope without saying what shape it is",
        'file': "src/mcp.rs",
        'find': '        "type": "string",\n        "description": format!(',
        'replace': '        "description": format!(',
        'tests': ["mcp"],
        'expect': "caught",
    },
    # --- naming what a number describes ---------------------------------------
    {
        # A substituted hit reports the *superseded* memory\'s score. Presenting it under the plain
        # name invites the caller to read it as describing the content beside it.
        'name': "view: present a substituted hit's score as if it were the memory's own",
        'file': "src/mcp.rs",
        'find': "                Some(replaces_matched) => FoundBy::SemanticSubstituted {\n                    matched_similarity: hit.similarity,\n                    inherited: hit.inherited,\n                    replaces_matched,\n                },",
        # Repaired 2026-08-26: `FoundBy::Semantic` gained `supersession_cycle` and
        # `superseded_beyond_this_set`, so this replacement stopped compiling.
        'replace': "                Some(_replaces_matched) => FoundBy::Semantic {\n                    similarity: hit.similarity,\n                    inherited: hit.inherited,\n                    supersession_cycle: hit.in_cycle,\n                    superseded_beyond_this_set: hit.superseded_beyond_this_set,\n                },",
        'tests': ["mcp"],
        'expect': "caught",
    },
    # --- what a returned memory carries ---------------------------------------
    {
        # The whole point of MemoryView is that a single field cannot go missing by accident, so
        # the mutation that matters is the one field the flattened memory does not supply.
        'name': "view: stop reporting the substitution behind a keyword hit",
        'file': "src/mcp.rs",
        'find': "                Some(replaces_matched) => FoundBy::KeywordSubstituted {\n                    matched_keywords: hit.matched_keywords,\n                    replaces_matched,\n                },",
        # Repaired 2026-08-26, same drift as its semantic sibling above.
        'replace': "                Some(_replaces_matched) => FoundBy::Keyword {\n                    matched_keywords: hit.matched_keywords,\n                    supersession_cycle: hit.in_cycle,\n                    superseded_beyond_this_set: hit.superseded_beyond_this_set,\n                },",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "view: return the memory's content alone instead of the whole memory",
        'file': "src/mcp.rs",
        'find': "    #[serde(flatten)]\n    memory: &'a Memory,",
        'replace': "    #[serde(skip)]\n    memory: &'a Memory,",
        'tests': ["mcp"],
        'expect': "caught",
    },
    # --- the empty corpus -----------------------------------------------------
    {
        'name': "centroid: divide by a zero count instead of bailing out",
        'file': "src/embedding.rs",
        'find': "    if count == 0 {\n        return None;\n    }\n",
        'replace': "",
        'tests': ["mcp", "recall", "cross_store"],
        'expect': "survives",
        'reason': """count is zero only when every store in the union is empty, and an empty union has no vectors
to score, so the NaN centroid this produces is never applied to anything. The guard is defensive
rather than load-bearing: it stops a NaN from existing at all, instead of relying on nothing
downstream ever reading one. Worth keeping and worth knowing no test can hold it — if the empty
union ever becomes a path that scores, this record is where that change of status should be
noticed.""",
    },
    {
        # A created store has to get the server's model, or every memory stored into it can never be found
        # by meaning. Anchored on the comment after it since `set-settled-at-first-write` step 3 removed the
        # one it used to follow.
        'name': 'sets: create a store without giving it the model',
        'file': 'src/mcp.rs',
        'find': '        self.add_store(store);\n\n        // **Nor is an author named here',
        'replace': '        self.stores.insert(scope.clone(), store);\n\n        // **Nor is an author named here',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # status reported this from the server's own embedder once, and disagreed with hoard_recall on
        # the same server. An approval snapshot caught it; this keeps it caught.
        'name': "sets: report searchability from the server rather than the store",
        'file': "src/mcp.rs",
        # Re-anchored 2026-08-09: the call became fallible and spans three lines.
        'find': "                \"semantic_search_available\": store\n                    .can_search_by_meaning()\n                    .map_err(|error| format!(\"could not read the model state: {error}\"))?,",
        'replace': "                \"semantic_search_available\": self.embedder.is_some(),",
        'tests': ["sets", "approval"],
        'expect': "caught",
    },
    {
        # Creating over a database that exists but is not loaded could hide every memory it holds.
        'name': "sets: create over an existing database",
        'file': "src/mcp.rs",
        'find': "        if path.exists() {\n            return Err(format!(\n                \"a database for the set {name:?} already exists",
        'replace': "        if false {\n            return Err(format!(\n                \"a database for the set {name:?} already exists",
        'tests': ["sets"],
        'expect': "survives",
        'reason': """No test covers it, and the reason is that provoking it needs a store on disk that the server was
never told about -- the tests build their servers explicitly, so every database in the data directory
is one they loaded. Reaching this branch means constructing a directory behind the server's back,
which no fixture does today. Recorded rather than fixed because the guard protects against hiding
memories, which is the expensive direction, and because the honest test for it belongs with `import`
and `restore` -- those are the operations that will make an unloaded-but-present database a normal
state rather than an oddity.""",
    },
    {
        # `no-tool-lists-loaded-scopes` was a Blocks defect precisely because this message named two
        # actions no caller could take. The fix is the wording, so the wording is what needs a guard.
        'name': "sets: stop pointing the unloaded-scope refusal at a usable action",
        'file': "src/mcp.rs",
        'find': "        \"no memory set is loaded for scope {}. Call `hoard_sets` with action \\\"status\\\" to see \\\n         which sets this machine holds, or action \\\"create\\\" to bring a new shared set into being.\",",
        'replace': "        \"no memory set is loaded for scope {}; check the set name.\",",
        'tests': ["sets", "approval"],
        'expect': "caught",
    },
    {
        # Without this the caller gets a filesystem error from deep inside, naming a path it never
        # typed, for a directory it was supposed to have created.
        'name': "export: accept a directory that does not exist",
        'file': "src/mcp.rs",
        'find': "        if !directory.is_dir() {",
        'replace': "        if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Nothing imports yet, so the base looks unobserved -- but known_targets reads the same rows,
        # so the unfamiliar-target note is what currently holds this write in place.
        'name': "export: write the file without recording the agreement",
        'file': "src/mcp.rs",
        'find': "        store\n            .set_bases(&target, &bases)",
        'replace': "        Ok::<(), crate::store::StoreError>(())\n            .map(|()| ())\n            .and(store.count().map(|_| ()))",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The format's ordering guarantee is what keeps a re-export from reading as a rewrite, and a
        # streaming writer cannot sort what it has not seen -- so the order has to come from here.
        'name': "store: stream memories without ordering them",
        'file': "src/store.rs",
        # Anchored through to the binding below it, because `memory_ids` runs the same query and
        # `find` takes the first match.
        'find': '            .prepare("SELECT id FROM memories ORDER BY id")?;\n        let ids: Vec<String> = statement',
        'replace': '            .prepare("SELECT id FROM memories")?;\n        let ids: Vec<String> = statement',
        'tests': ["sets"],
        'expect': "survives",
        'reason': """SQLite returns rows of a table whose primary key is the id in primary-key order when no ORDER BY
is given, so the mutation produces the same sequence for every case the tests construct. The clause
is still load-bearing: that behaviour is an implementation detail of the query planner rather than a
guarantee, and it stops holding as soon as the query gains a join, an index scan, or a WHERE that the
planner satisfies a different way. Recorded rather than tested because the honest test would have to
provoke a different plan, which means writing a query this code does not use.""",
    },
    {
        # Losing this makes an import overwrite unexported local work -- the exact data-loss defect the
        # whole reconciliation design exists to prevent.
        'name': "reconcile: take the file when only the store changed",
        'file': "src/reconcile.rs",
        'find': "            (true, false) => Outcome::KeepLocal,",
        'replace': "            (true, false) => Outcome::TakeFile,",
        'tests': ["reconcile", "sets"],
        'expect': "caught",
    },
    {
        # Without the convergence case, two people making the same correction are reported as
        # disagreeing, and "conflict" stops meaning anything.
        'name': "reconcile: call convergence a conflict",
        'file': "src/reconcile.rs",
        'find': "                if file == stored {\n                    Outcome::Converged\n                } else {\n                    Outcome::Conflict\n                }",
        'replace': "                Outcome::Conflict",
        'tests': ["reconcile", "sets"],
        'expect': "caught",
    },
    {
        # Resolving a conflict by silently preferring one side is the failure the design refuses.
        'name': "reconcile: resolve a conflict by taking the file",
        'file': "src/reconcile.rs",
        'find': "            } else {\n                Outcome::Conflict\n            }\n        }\n\n        (Some(base), Some(stored))",
        'replace': "            } else {\n                Outcome::TakeFile\n            }\n        }\n\n        (Some(base), Some(stored))",
        'tests': ["reconcile", "sets"],
        'expect': "caught",
    },
    {
        # An import agrees about only some records. Replacing the whole target's bases would move the
        # base for a conflicted record, which is how a conflict gets silently accepted next time.
        'name': "import: replace every base instead of merging the agreed ones",
        'file': "src/mcp.rs",
        'find': "            .merge_bases(&target, &agreed)",
        'replace': "            .set_bases(&target, &agreed)",
        'tests': ["sets"],
        'expect': "survives",
        'reason': """The two differ only for a record that HAS a recorded base and then diverges locally -- set_bases would
delete that base, and the next import would see no ancestor, two differing sides, and escalate a
settled kept-local record to a conflict. That state cannot be produced through the tool surface,
because nothing can modify a memory that already exists: hoard_memorize always mints a new id. So
every reachable kept-local record has no base to lose, and every reachable conflict already has no
base. Tracked as no-tool-can-modify-a-memory in TRACKING.md, which is the same gap that keeps the
both-sides-edited conflict branch unreachable from a client. merge_bases is still the correct
operation -- an import agrees about only some records -- and this record is where its guard should
become testable once amendment exists.""",
    },
    {
        # Validating before applying is what keeps a malformed last line from leaving the store
        # half-updated.
        'name': "import: skip the validating pass",
        'file': "src/mcp.rs",
        'find': "            memory_file::validate(&expectation, file)",
        'replace': "            memory_file::validate(&expectation, std::io::empty()).map(|_| ()).and_then(|()| memory_file::validate(&expectation, file))",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A base is not a property of the memory row. INSERT OR REPLACE deletes the old row, so a
        # cascade here destroys the agreement every time a memory is rewritten -- which turns a
        # one-sided amendment into a conflict on the next import.
        'name': "store: let a rewritten memory cascade away its base",
        'file': "src/store.rs",
        'find': "             CREATE TABLE IF NOT EXISTS bases (\n                 target    TEXT NOT NULL,\n                 memory_id TEXT NOT NULL,",
        'replace': "             CREATE TABLE IF NOT EXISTS bases (\n                 target    TEXT NOT NULL,\n                 memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,",
        'tests': ["sets"],
        'expect': "survives",
        'reason': """Survives because the migration repairs it, which is the migration proving it works rather than a gap
in the guard. drop_cascading_bases runs after the schema batch, detects a foreign key on bases, and
rebuilds the table without one -- so putting the cascade back in CREATE TABLE is undone before any
test runs. The behaviour IS covered: a_memory_changed_on_one_side_only_is_not_a_conflict is what
found this defect and fails if the cascade survives into a live store.

What is genuinely untested is the migration against a store that predates the fix. add_folded_keywords
has that test, built on the rusqlite dev-dependency so a database this code did not write can be
constructed; bases does not. Worth adding, and cheap -- the fixture already exists for the keyword
migration.""",
    },
    {
        # The whole of correction-gated-on-service in one line: a record that has entered service is
        # refused. Inverted, correction reaches exactly the records that must be superseded instead,
        # and lets through the one case a merge cannot reason about.
        'name': "correct: invert the service check, so only exported records are correctable",
        'file': "src/mcp.rs",
        'find': "        if store.has_base(params.id).map_err(|error| {",
        'replace': "        if !store.has_base(params.id).map_err(|error| {",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # `amending-a-claim-is-refused-where-no-client-can-reach-it` reproduced in a new place: the
        # handler accepts the field, the schema does not name it, and a validating client strips it
        # before it is sent. Every handler-level test would stay green, because they build the
        # request themselves -- which is why the correct.rs tests go through the published
        # properties instead.
        'name': "correct: accept content without declaring it in the schema",
        'file': "src/mcp.rs",
        'find': '                    "content": {\n                        "type": "string",\n                        "description": "Replaces the memory itself. Reaches only a memory that has \\\n                                        never been exported -- nobody has read that one, so there \\\n                                        is no belief to destroy.",\n                    },',
        'replace': "",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # `tombstone-carries-the-checksum-it-forgot`. Without the sequential case a forget arriving
        # with no ancestor is a conflict for a reason that is true and useless: a tombstone's
        # checksum never equals a memory's, so *both sides changed* always holds.
        'name': "reconcile: never treat a forget of the version we hold as sequential",
        'file': "src/reconcile.rs",
        'find': '        (Some(forgot), Some(stored)) if forgot == stored => Outcome::TakeFile,',
        'replace': "        (Some(forgot), Some(stored)) if forgot != stored => Outcome::TakeFile,",
        'tests': ["sets", "reconcile"],
        'expect': "caught",
    },
    {
        # The other direction, and the dangerous one: applying a forget of a version this store
        # never had silently discards work nobody agreed to discard.
        'name': "reconcile: apply any forget when there is no base",
        'file': "src/reconcile.rs",
        'find': '        (Some(forgot), Some(stored)) if forgot == stored => Outcome::TakeFile,',
        'replace': "        (Some(_), Some(_)) => Outcome::TakeFile,",
        'tests': ["sets", "reconcile"],
        'expect': "caught",
    },
    {
        # A recorded agreement outranks the sender's claim about what it removed. Inverted, a
        # tombstone's own field overrides an ancestor both sides established.
        'name': "reconcile: let the forgotten checksum override a recorded base",
        'file': "src/reconcile.rs",
        'find': '    let ordinary = resolve(file, base, stored);\n    if base.is_some() {\n        return ordinary;\n    }',
        'replace': """    let ordinary = resolve(file, base, stored);
    if base.is_none() {
        return ordinary;
    }""",
        'tests': ["sets", "reconcile"],
        'expect': "caught",
    },
    {
        # The field is only worth anything if `forget` fills it in. Recording nothing leaves every
        # tombstone unanswerable, which is the state this work removed.
        'name': "forget: record no checksum for what was forgotten",
        'file': "src/store.rs",
        'find': '        let forgot = self.get(id)?.map(|memory| {\n            crate::checksum::of_record(&crate::memory_file::Record::Memory(Box::new(memory)))\n        });',
        'replace': "        let forgot: Option<u64> = None;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `forgetting-a-chains-middle-makes-a-stale-conclusion-current-again`. Drop the ancestry and
        # a chain silently loses its middle: a twice-replaced conclusion reports itself as current,
        # with no artifact anywhere to notice, which is what made this Corrupts rather than Misleads.
        'name': "forget: keep no ancestry, so a chain loses its middle",
        'file': "src/memory_file.rs",
        'find': '            .filter(|link| matches!(link.label, LinkLabel::Supersedes | LinkLabel::SupersededBy))',
        'replace': "            .filter(|link| matches!(link.label, LinkLabel::Resolves))",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The other direction, and the one that would leak: keeping every label means a tombstone
        # carries the forgotten memory's judgements about other memories, which is content.
        'name': "forget: keep every link, not just the ancestry",
        'file': "src/memory_file.rs",
        'find': '            .filter(|link| matches!(link.label, LinkLabel::Supersedes | LinkLabel::SupersededBy))',
        'replace': "            .filter(|_| true)",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # A cross-set target's keywords are free text written by the same author as the content
        # being removed, so they go with it. This is the one place the ancestry could carry prose.
        'name': "forget: keep a cross-set target's keywords on the tombstone",
        'file': "src/memory_file.rs",
        'find': "                        set, id, set_id, ..\n                    } => LinkTarget::CrossSet {\n                        set,\n                        id,\n                        keywords: Vec::new(),",
        'replace': "                        set, id, set_id, keywords\n                    } => LinkTarget::CrossSet {\n                        set,\n                        id,\n                        keywords,",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The fix has to survive the file, which is the only way a set reaches another machine.
        # Deriving the ancestry locally derives none on a machine that never held the memory.
        'name': "import: forget an arriving tombstone locally, losing its ancestry",
        'file': "src/mcp.rs",
        'find': '                        memory_file::Record::Tombstone(tombstone) => store\n                            .accept_tombstone(tombstone)',
        'replace': """                        memory_file::Record::Tombstone(tombstone) => store
                            .forget(tombstone.id, tombstone.forgotten.by, tombstone.forgotten.at)""",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The remark exists to say a judgement may not have survived the text moving. Never firing
        # is the failure it was built against; the pair below is what tells the two apart, since a
        # remark that always fires and one that never fires are equally useless.
        'name': "correct: never remark on what points at a rewritten memory",
        'file': "src/mcp.rs",
        'find': '            .any(|field| matches!(*field, "content" | "why"))',
        'replace': "            .any(|field| matches!(*field, \"nothing\" | \"at all\"))",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The other direction: unscoped, it fires on every confidence change and teaches callers to
        # skip it, which costs the cases it exists for.
        'name': "correct: remark on in-links after an apparatus-only amendment",
        'file': "src/mcp.rs",
        'find': '            .any(|field| matches!(*field, "content" | "why"))',
        'replace': "            .any(|field| !field.is_empty())",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # A supersedes edge says this memory was replaced, which stays true however it reads. Left
        # in, the remark asks the caller to re-judge a relationship the rewrite cannot have touched.
        'name': "correct: include supersedes edges in the in-link remark",
        'file': "src/mcp.rs",
        'find': '                .filter(|edge| edge.label != LinkLabel::Supersedes)',
        'replace': "                .filter(|edge| edge.label != LinkLabel::Resolves)",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # Keywords are part of the vectorised text, so an amendment that did not re-embed would leave
        # the memory findable by the spelling it no longer has.
        'name': "correct: amend without updating the modified attribution",
        'file': "src/mcp.rs",
        'find': "        memory.modified = Attribution {\n            by: author,",
        'replace': "        let _unused = Attribution {\n            by: author,",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The whole point of a tombstone: absence means nothing, deletion is a line. If the writer
        # drops forgotten records, forgetting never reaches another machine.
        #
        # Re-anchored 2026-08-25, when the loop moved into `serialise_set` -- which is also what
        # widened it: one writer now serves `export` and the safety copy `drop` writes, so this
        # proves the tombstones reach both. A backup that silently drops what a set forgot would
        # resurrect it on restore.
        'name': "serialise: write only memories, leaving tombstones behind",
        'file': "src/mcp.rs",
        'find': "            .stream_records(|record| {\n                let from = body.len();",
        'replace': "            .stream_memories(|memory| {\n                let record = &crate::memory_file::Record::Memory(Box::new(memory.clone()));\n                let from = body.len();",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Reading only memory rows makes a memory this machine already forgot look absent, so the
        # matching tombstone in the file reads as a record that vanished -- an anomaly reported on
        # every import of a set that is perfectly in agreement.
        'name': "import: read the store's memories but not its tombstones",
        'file': "src/mcp.rs",
        # Re-anchored 2026-08-26: the map gained a timestamp beside the checksum when
        # `import: take a record backwards without saying so` was built.
        'find': "                stored.insert(\n                    record.id(),\n                    (crate::checksum::of_record(record), record.as_of()),\n                );",
        'replace': "                if matches!(record, crate::memory_file::Record::Memory(_)) {\n                    stored.insert(\n                        record.id(),\n                        (crate::checksum::of_record(record), record.as_of()),\n                    );\n                }",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Forgetting exists partly to remove sensitive data, so the row has to go rather than be
        # flagged -- a kept row still holds the content, the keywords and the vector.
        'name': "store: forget without deleting the memory",
        'file': "src/store.rs",
        'find': '            "DELETE FROM memories WHERE id = ?1",\n            params![id.to_string()],',
        'replace': '            "SELECT 1 WHERE ?1 IS NULL",\n            params![id.to_string()],',
        'tests': ["correct", "sets"],
        'expect': "caught",
    },
    {
        # Serialising a struct emits declaration order, so this makes the checksum depend on where a
        # field sits in a source file -- reordering Memory's fields would then invalidate every base on
        # every machine, silently. The test this replaced was vacuous and did not catch it.
        'name': "memory_file: canonicalise in declaration order instead of sorted",
        'file': "src/memory_file.rs",
        'find': "        serde_json::to_string(&value).expect(SERIALISES)",
        'replace': "        match self {\n            Self::Memory(memory) => serde_json::to_string(memory).expect(SERIALISES),\n            Self::Tombstone(tombstone) => serde_json::to_string(tombstone).expect(SERIALISES),\n        }",
        'tests': ["checksum"],
        'expect': "caught",
    },
    {
        # WAL is a property of the file, so a store left on the default rollback journal makes a
        # writer in one server lock out every reader in another. Nothing in a single-process test
        # can see the difference, which is why this needs a mutation to prove it is checked at all.
        'name': "store: leave the store file on the default journal mode",
        'file': "src/store.rs",
        'find': '        connection.query_row("PRAGMA journal_mode = WAL", [], |row| {',
        'replace': '        connection.query_row("PRAGMA journal_mode", [], |row| {',
        'tests': ["concurrency"],
        'expect': "caught",
    },
    {
        # The whole cache-coherence fix in one line. Without the refresh, a memory another process
        # stored is invisible to semantic recall until this one restarts -- while keyword search,
        # which reads SQL, finds it.
        'name': "store: recall from the cache without refreshing it",
        'file': "src/store.rs",
        # Re-pointed 2026-08-18: every semantic recall now runs through `recall_across`, which
        # refreshes each store in its own pass before any of them is scanned.
        'find': "    for store in stores.iter_mut() {\n        store.prepare()?;\n    }",
        'replace': "",
        'tests': ["concurrency", "recall"],
        'expect': "caught",
    },
    {
        # Same guard, reached the other way: `recall_across` is the path the MCP surface actually
        # takes, so a fix that covered only `Store::recall` would leave every real call stale.
        'name': "store: recall across stores without refreshing them",
        'file': "src/store.rs",
        # Re-pointed 2026-08-08, with the same cause as the one above.
        'find': "    for store in stores.iter_mut() {\n        store.prepare()?;\n    }",
        'replace': "",
        'tests': ["concurrency", "cross_store", "sets"],
        'expect': "caught",
    },
    {
        # The distinction the whole fix rests on. Answering from the embedder is right in every
        # case anyone pictures and wrong for a memory written before the model arrived -- which is
        # `project-stores-get-no-embedder` one layer down, and the reason this asks the database.
        'name': "store: report findability from the capability, not the vector",
        'file': "src/store.rs",
        # Re-anchored 2026-09-11: the query now asks for a real vector, since a model's verdict that
        # there is nothing to represent is a row too.
        'find': '''        let found: Option<i64> = self
            .connection
            .query_row(
                "SELECT 1 FROM vectors WHERE memory_id = ?1 AND length(vector) > 0",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;''',
        'replace': "        let _ = id;\n        let found: Option<i64> = self.embedder.as_ref().map(|_| 1);",
        'tests': ["recall", "mcp"],
        'expect': "caught",
    },
    {
        # Silence is the original defect: the memory is stored, the response says nothing, and a
        # memory that can never be found by meaning looks exactly like one that can.
        'name': "memorize: stop saying whether the memory can be found",
        'file': "src/mcp.rs",
        'find': '            "findable_by_meaning": findable,\n',
        'replace': "",
        'tests': ["mcp", "approval"],
        'expect': "caught",
    },
    {
        # Without the record there is nothing to compare a loaded model against, and every defect
        # in this area is the absence of that comparison rather than a wrong answer from it.
        'name': "store: stop recording which model produced the vectors",
        'file': "src/store.rs",
        # Re-anchored 2026-08-09: the write moved into `record_model`, which also keeps the
        # displaced identity so an alternation can be recognised. Re-anchored 2026-09-11: every row
        # now enters `vectors` through `persist`, so this is the one call left to remove.
        'find': "            self.record_model(&embeddings.model)?;",
        'replace': "            let _ = &embeddings.model;",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # The comparison itself. Answering "is there an embedder?" is the question the code asked
        # before, and it cannot tell a store whose model is right from one whose model is merely
        # present -- which is the swapped-model case, where cosine still computes and lies.
        'name': "store: treat any loaded model as the right one",
        'file': "src/store.rs",
        'find': "        if loaded.as_ref() == Some(&recorded) {",
        'replace': "        if loaded.is_some() {",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # Identity by path reports a replaced model file as the same model, which is precisely the
        # substitution being guarded against.
        'name': "embedder: identify a model by where it is rather than what it is",
        'file': "src/embedder.rs",
        'find': "    let bytes = std::fs::read(path)?;",
        'replace': "    let bytes = path.to_string_lossy().into_owned().into_bytes();",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # Keeping the old model's vectors is the mixture the design forbids: two vector spaces in
        # one store, ranked against each other, computing perfectly and meaning nothing.
        #
        # Re-anchored 2026-08-09. A read no longer discards them at all — that was the loop two
        # servers got into — so the discard this guards now lives in the explicit rebuild, which is
        # the one place a model change is sanctioned.
        'name': "store: keep the old vectors when the model changes",
        'file': "src/store.rs",
        'find': "        self.connection.execute(\"DELETE FROM vectors\", [])?;\n        self.vectors = VectorSet::new(0);\n        // Before embedding, not after",
        'replace': "        // Before embedding, not after",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # The repair itself. Without it a memory stored before the model arrived is unfindable by
        # meaning for good, which is the ordinary onboarding path rather than an edge case.
        'name': "store: never fill in a missing embedding",
        'file': "src/store.rs",
        'find': "        let memories: Vec<&Memory> = missing.iter().collect();\n        self.embed_all(&memories)?;",
        'replace': "",
        'tests': ["model_identity", "recall"],
        'expect': "caught",
    },
    {
        # Letting the destructive rewrite through is the defect exactly as filed: one amendment
        # removes a memory from semantic recall permanently, silently, with no way back.
        'name': "store: let a rewrite destroy an embedding nothing can rebuild",
        'file': "src/store.rs",
        'find': "        self.is_findable_by_meaning(memory.id)\n    }",
        'replace': "        let _ = memory;\n        Ok(false)\n    }",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # The opposite error, and the reason the guard asks about the vector rather than the model:
        # refusing every write without a model turns a missing model into a read-only store, which
        # the loud-degradation rule specifically does not ask for.
        'name': "store: refuse every write when no model is loaded",
        'file': "src/store.rs",
        'find': "        self.is_findable_by_meaning(memory.id)\n    }",
        'replace': "        let _ = memory;\n        Ok(true)\n    }",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        'name': "store: stop asking writes to wait for a lock",
        'file': "src/store.rs",
        'find': "        connection.busy_timeout(LOCK_WAIT)?;",
        'replace': "",
        'tests': ["concurrency"],
        'expect': "survives",
        'reason': (
            "`rusqlite::Connection::open` already sets a five-second busy timeout of its own, so "
            "removing ours changes nothing observable and no test can go red. Recorded rather than "
            "deleted because that fact is the whole point: the defect this fixes was filed from "
            "reading `from_connection` and finding one pragma, and SQLite's own default is indeed "
            "zero -- the waiting behaviour came from the driver, not from anything this project "
            "decided. The line stays so the requirement belongs to the store rather than to a "
            "dependency's default, and this entry is what will fail loudly if that default ever "
            "changes and someone assumes the mutation should now be caught."
        ),
    },
    {
        # Reloading without discarding first is what `forget` did for months: it only ever added, so
        # a departed memory's vector stayed behind and ate the scan width that a survivor needed.
        'name': "store: reload vectors without discarding the old set",
        'file': "src/store.rs",
        'find': "        self.vectors = VectorSet::new(0);\n        for (id, blob) in rows {",
        'replace': "        for (id, blob) in rows {",
        'tests': ["recall", "concurrency"],
        'expect': "caught",
    },
    {
        # The defect this restores: a store with vectors and no recorded model reported itself
        # Unembedded, which promises "there are no vectors for a write to invalidate" while holding
        # 26 of them. Worth a mutation because the guard is one comparison against zero and the
        # failure it prevents is silent -- nothing errors, the mismatch detector simply never fires.
        'name': "store: call every unrecorded model an unembedded store",
        'file': "src/store.rs",
        'find': "            return Ok(if vectors == 0 {",
        'replace': "            return Ok(if true {",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # The other half: detecting the state but declining to act on it. Adopting unaccounted-for
        # vectors writes this model's identity over vectors it did not produce, which is a guess
        # made permanent -- and it reads as correct, because the store afterwards looks Current.
        'name': "store: search vectors whose provenance is unknown",
        'file': "src/store.rs",
        # Re-anchored 2026-08-09: the two states stopped sharing an arm when a mismatch became
        # something a read leaves alone.
        'find': "            ModelState::Mismatched { .. } | ModelState::ProvenanceUnknown { .. }",
        'replace': "            ModelState::Mismatched { .. }",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # status reporting a state but not the identity is the shape that already failed twice --
        # a server-wide flag in project-stores-get-no-embedder, then can_search_by_meaning saying
        # an embedder exists rather than that this store can be searched. A caller comparing two
        # servers needs the name, and "current" without it is the same non-answer in a third form.
        'name': "mcp: report a current store's state without naming its model",
        'file': "src/mcp.rs",
        'find': '            "state": "current",\n            "recorded": model.as_str(),',
        'replace': '            "state": "current",\n            "recorded": Value::Null,',
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Swapping these reads as correct -- both fields are populated, both are real identities --
        # and tells the caller the store holds vectors from the model it is actually searching with,
        # which is the exact inversion of the fact status exists to report.
        'name': "mcp: swap the recorded and loaded models in a mismatch report",
        'file': "src/mcp.rs",
        'find': '            "recorded": recorded.as_str(),\n            "loaded": loaded.as_str(),',
        'replace': '            "recorded": loaded.as_str(),\n            "loaded": recorded.as_str(),',
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Scanning a store whose vectors came from another model. The arithmetic works, the scores
        # look ordinary, and the ranking is computed across two embedding spaces -- the failure that
        # produced a confidently wrong answer on 2026-08-09 with nothing in the payload to show it.
        #
        # It needed a test written for it: the first version of this mutation survived, because the
        # test asserted what the store *was* afterwards and never that recall returned nothing.
        #
        # Re-pointed 2026-08-18 onto the filter in `recall_across`, which is now the only place a
        # store is kept out of the scan.
        # Repaired 2026-08-26: an `else` branch was added after this was written -- the one that
        # names each skipped set and why -- so deleting the `if` left the `else` dangling and the
        # mutation stopped compiling. Inverting the condition reaches the same behaviour (every
        # store searched, whatever model its vectors came from) and leaves the branch intact.
        'name': "store: search a store whose vectors came from another model",
        'file': "src/store.rs",
        'find': "        if store.can_search_by_meaning()? {\n            searchable.push(*store);",
        'replace': "        if store.can_search_by_meaning()? || true {\n            searchable.push(*store);",
        'tests': ["model_identity", "sets"],
        'expect': "caught",
    },
    {
        # Embedding a new memory into a mismatched store, which is how a store comes to hold two
        # models' vectors at once -- the mixture `one store, one model` exists to make impossible.
        # This is the only guard for it: an earlier duplicate in `reconcile_embeddings` was removed
        # after a mutation showed it changed nothing.
        'name': "store: embed into a store that belongs to a different model",
        'file': "src/store.rs",
        'find': "        if matches!(self.model_state()?, ModelState::Mismatched { .. }) {\n            return Ok(None);\n        }",
        'replace': "",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # The defect itself: attribution taken from the process rather than from the store. Observed
        # on 2026-08-10 against two real servers, one `local` store holding two identities for one
        # person. Re-pointed 2026-08-31 when `by` became a reference: the property moved from
        # *which name* to *which record*, and minting unconditionally is now how the process would
        # win over the store -- the same defect, one indirection along.
        'name': "store: mint a fresh identity on every write instead of reading the store's",
        'file': "src/store.rs",
        'find': "        if let Some(cached) = self.author_record()? {\n            return Ok(Some(cached));\n        }\n\n        if self.holds_any_author_record()? {\n            return Ok(None);\n        }\n",
        'replace': "",
        'tests': ["store", "concurrency", "sets"],
        'expect': "caught",
    },
    {
        # Setting an author must not quietly rewrite what is already stored. The count is the only
        # thing that makes the split visible at the moment it is created, and a zero would read as
        # reassurance.
        # The two directions of the placement rule, mutated separately because they fail differently:
        # ignoring the common directory leaves each checkout with its own personal memory, while
        # creating shared sets there makes every new set machine-bound whether or not it should be.
        'name': "layout: ignore the common directory and keep everything in the home",
        'file': "src/store.rs",
        'find': "        self.common.as_deref().unwrap_or(&self.home)",
        'replace': "        &self.home",
        'tests': ["store", "sets"],
        'expect': "caught",
    },
    {
        'name': "layout: create new shared sets in the common directory",
        'file': "src/store.rs",
        'find': "            Scope::Local => self.common(),\n            Scope::Shared(_) => &self.home,",
        'replace': "            Scope::Local => self.common(),\n            Scope::Shared(_) => self.common(),",
        'tests': ["store", "sets"],
        'expect': "caught",
    },
    {
        # Which copy wins when a set is in both directories. Letting the home win puts a stale or
        # empty store in front of real memories -- the shape of
        # moving-the-local-store-leaves-the-old-one-unmentioned, arrived at a second way.
        'name': "layout: let the home copy win over the common one",
        'file': "src/store.rs",
        'find': "            vec![self.common(), &self.home]",
        'replace': "            vec![&self.home, self.common()]",
        'tests': ["store", "sets"],
        'expect': "caught",
    },
    {
        # Scanning one directory twice when both names reach it, which makes every set shadow itself
        # in the *default* configuration -- the bug this distinction was introduced to fix, and one
        # that a test using two real directories would never see.
        'name': "layout: scan both names even when they reach one directory",
        'file': "src/store.rs",
        'find': "        let directories: Vec<&Path> = if self.directories_differ() {",
        'replace': "        let directories: Vec<&Path> = if true {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # Reporting a redundantly declared common directory. The note is the only signal that a
        # deliberate setting is doing nothing.
        'name': "layout: stay silent about a common directory that is the home",
        'file': "src/store.rs",
        'find': "            .is_some_and(|common| same_directory(common, &self.home))",
        'replace': "            .is_some_and(|_| false)",
        'tests': ["store", "sets"],
        'expect': "caught",
    },
    {
        # An in-memory store reports its path as "", which would reach a caller looking like a
        # filesystem answer rather than the absence of one.
        'name': "store: report an empty path as though it were a file",
        'file': "src/store.rs",
        'find': "            .filter(|path| !path.is_empty())",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Re-attributing imported records to the local identity. The mistake looks like tidiness --
        # `memorize` and `amend` both stamp the local author, so an import that does not can read
        # like an oversight -- and it would silently rewrite the authorship of every memory arriving
        # from anyone else. It would also foreclose export-edit-import, which is the only way to
        # re-attribute memories in bulk and is the procedure store-registry.md points at.
        'name': "mcp: re-attribute imported records to the importing store",
        'file': "src/mcp.rs",
        'find': "                            store\n                                .accept_from_file(memory)",
        'replace': "                            let mut memory = (**memory).clone();\n                            if let Ok(Some(local)) = store.author_record() {\n                                memory.created.by = local;\n                                memory.modified.by = local;\n                            }\n                            store\n                                .accept_from_file(&memory)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The whole of `oneof-tool-schemas-present-no-parameters`. Publishing only `oneOf` is legal
        # and left the management surface uncallable from a client that reads `properties`.
        'name': "mcp: publish the branches without a flat view",
        'file': "src/mcp.rs",
        'find': '    schema["properties"] = Value::Object(properties);',
        'replace': "    let _ = properties;",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The second client failure mode: branches merged, `action` pinned to the first one. Callable
        # and unable to express any action but `status`, which is worse than being uncallable because
        # nothing reports it.
        'name': "mcp: offer only the first action in the flat view",
        'file': "src/mcp.rs",
        'find': '            "enum": actions,',
        'replace': '            "enum": actions.first().cloned().into_iter().collect::<Vec<_>>(),',
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Collapsing the two states back together: a set holding memories told that nothing has been
        # written to it, beside the count that says otherwise.
        'name': 'mcp: deny writes to any set with no recorded author',
        'file': 'src/mcp.rs',
        'find': '    let state = if memories == 0 {',
        'replace': '    let state = if true {',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # One reason given for two unrelated skips. The local case and the unbuilt-scan case both
        # report `false`, and this is the sentence that tells them apart.
        'name': "mcp: give one reason for skipping the prohibited-data scan",
        'file': "src/mcp.rs",
        'find': "            \"prohibited_data_note\": if scope.exports_to().is_none() {",
        'replace': "            \"prohibited_data_note\": if true {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Leaving the branches on the wire. The API refuses a top-level oneOf and fails the entire
        # tool list for it -- every tool from every registered server, until the session restarts --
        # so this is the one schema mistake whose blast radius is other people's servers.
        'name': "mcp: leave the branches on the wire beside the flat view",
        'file': "src/mcp.rs",
        'find': '        object.remove("oneOf");',
        'replace': '        object.remove("notAKeyword");',
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Publishing an alternation again, anywhere. The client that empties it leaves the property
        # with no type, after which an object argument arrives as text and no shared set can be
        # named at all.
        'name': "mcp: publish scope as an alternation again",
        'file': "src/mcp.rs",
        'find': '        "type": "string",\n        "description": format!(\n            "{description} Either `{local}`',
        'replace': '        "oneOf": [{"const": local.clone()}, {"type": "object"}],\n        "description": format!(\n            "{description} Either `{local}`',
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Dropping the object spelling of a scope. This is the mutation that matters most, because
        # it is what tidying looks like: the schema no longer advertises the object form, so a
        # reader may take the parser's acceptance of it for dead code. It is what memory files hold
        # and what a client preserving alternations still sends.
        'name': "scope: stop accepting the object spelling",
        'file': "src/memory/scope.rs",
        'find': '            "shared" => Scope::Shared(map.next_value::<SetName>()?),',
        'replace': '            "shared" => return Err(M::Error::custom("no")),',
        'tests': ["memory_model", "mcp", "sets"],
        'expect': "caught",
    },
    {
        # And the other direction: dropping the scalar spelling, which is the one now published.
        'name': "scope: stop accepting the scalar spelling",
        'file': "src/memory/scope.rs",
        'find': '        match value.strip_prefix(SHARED_PREFIX) {',
        'replace': '        match None::<&str> {',
        'tests': ["memory_model", "store"],
        'expect': "caught",
    },
    {
        # Dropping the flat link target. Same shape as the scope mutation above and the same risk:
        # the nested form still parses, so losing this fails only for a client that cannot send one.
        'name': "link: stop accepting the flat target spelling",
        'file': "src/memory/link.rs",
        'find': '        if wire.target_set.is_none() && wire.target_keywords.is_none() {',
        'replace': '        if true {',
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # The budget is enforced by one test and nothing else, and that test passes whenever the
        # catalog is under the constant -- which is the shape a vacuous assertion also has. Cutting
        # the constant below the current catalog is what distinguishes them: a test that really
        # reads `MAX_CATALOG_CHARS` fails, and one that compares the catalog against itself does
        # not. Recorded when the budget was raised to 16,000, since raising a limit is exactly when
        # you want to know the limit is load-bearing. Re-anchored on 2026-08-13 when the budget
        # went to 18,000: a mutation keyed to a literal breaks silently when that literal moves,
        # and the runner reporting NOT APPLIED is the only thing that says so.
        'name': "conventions: cut the catalog budget below the catalog",
        'file': "src/conventions/mod.rs",
        'find': "pub const MAX_CATALOG_CHARS: usize = 18_000;",
        'replace': "pub const MAX_CATALOG_CHARS: usize = 1_000;",
        'tests': ["conventions"],
        'expect': "caught",
    },
{
        # The whole harness turns on reporting a percentile rather than a rank, and the two differ
        # only by a division -- so a test asserting a rank would pass against either. These pin the
        # two ways the percentile can quietly become the wrong number: divided by the ranked
        # population instead of the corpus, and not divided at all.
        'name': "eval: divide the percentile by the ranked population, not the corpus",
        'file': "src/eval.rs",
        'find': "                percentile: rank as f64 / corpus.size() as f64,",
        'replace': "                percentile: rank as f64 / (corpus.size() - 1) as f64,",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        'name': "eval: report the rank where the percentile belongs",
        'file': "src/eval.rs",
        'find': "                percentile: rank as f64 / corpus.size() as f64,",
        'replace': "                percentile: rank as f64,",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # Supersession is a validity relation, so validity resolution never traverses it and it is
        # not part of the question the measurement asks. It is also the most content-similar edge in
        # any corpus, since a superseding memory is supposed to resemble its predecessor -- so
        # including it would not break a run, it would quietly improve the result.
        #
        # Moved 2026-08-18 with the rule itself: the walk is shared between recall and the harness,
        # so this one line now governs both.
        'name': "traversal: treat supersession as traversable",
        'file': "src/traversal.rs",
        'find': '        LinkLabel::Supersedes | LinkLabel::SupersededBy | LinkLabel::Unrecognised(_) => false,',
        'replace': '        LinkLabel::Supersedes | LinkLabel::SupersededBy => true,\n        LinkLabel::Unrecognised(_) => false,',
        'tests': ["eval", "traversal", "recall"],
        'expect': "caught",
    },
    {
        # A cross-set target names a set this corpus is not. Resolving it by id alone would measure
        # a relationship that does not exist here, and would do it silently, because the id is
        # often present -- a corpus extracted from one project's documents reuses its own ids.
        'name': "eval: resolve a cross-set target by id alone",
        'file': "src/eval.rs",
        'find': "            if link.target.set().is_some() {\n                continue;\n            }",
        'replace': "",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # The source stands in for the query, and a query cannot retrieve itself. Leaving it in the
        # ranking shifts every rank by exactly one, which is invisible in a large corpus and is
        # precisely the size of the effect being measured at the top of the distribution.
        'name': "eval: leave the source in its own ranking",
        'file': "src/eval.rs",
        'find': "                    .filter(|hit| hit.id != memory.id)",
        'replace': "",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # Centroid removal is what the product's recall does, and it is worth 5.8 points of
        # recall@5 and half the worst hub. A harness scanning uncentred measures a pipeline nobody
        # runs, and every number it reports stays plausible.
        'name': "eval: score without removing the centroid",
        'file': "src/eval.rs",
        'find': "                    .nearest(&query, corpus.size())",
        'replace': "                    .nearest_uncentred(&query, corpus.size())",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # An unmeasurable edge has no percentile to contribute. Folding it into the median as a
        # zero would drag the reported figure toward the top of the distribution -- the direction
        # that makes traversal look redundant, and that argues for a smaller corpus.
        'name': "eval: count unmeasurable edges as measured",
        'file': "src/eval.rs",
        'find': "        measured: reaches.len(),",
        'replace': "        measured: reaches.len() + unmeasurable_edges(corpus),",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # The label diagnostic exists to answer whether an answer key carries information the
        # ranker does not already have. Scoring it uncentred measures a pipeline the product does
        # not run, and the answer stays plausible either way -- which is exactly the shape that
        # makes a wrong diagnostic worse than none.
        'name': "eval: rank labels against the query without removing the centroid",
        'file': "src/eval.rs",
        # Keyed to the binding as well as the call, because `rank_scored` and `ranks_from_query`
        # now share the call line -- and when they did, this mutation silently moved onto the other
        # function and passed. A literal that stops being unique stops testing what it names.
        'find': "    let ranking: Vec<MemoryId> = corpus\n        .vectors\n        .nearest(query, corpus.size())",
        'replace': "    let ranking: Vec<MemoryId> = corpus\n        .vectors\n        .nearest_uncentred(query, corpus.size())",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # An id absent from the corpus has no rank. Reporting it as last would push the measured
        # distribution outward -- toward "these labels reach far past content similarity" -- which
        # is the reassuring direction and the one nobody would question.
        'name': "eval: give an unknown id the last rank instead of none",
        'file': "src/eval.rs",
        'find': "        .filter_map(|id| ranking.iter().position(|other| other == id))",
        'replace': "        .map(|id| ranking.iter().position(|other| other == id).unwrap_or(ranking.len()))",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # Everything the tuning reports goes through `rank_scored`, and its centring was unpinned
        # until the mutation above wandered onto it. Centred and uncentred both produce a plausible
        # ranking, so nothing about a wrong one looks wrong.
        'name': "eval: rank for tuning without removing the centroid",
        'file': "src/eval.rs",
        # Re-anchored 2026-08-18 for the **third** time, on the binding rather than on the scan call:
        # `ranks_from_query` runs the identical `.nearest(query, corpus.size())` two hundred lines
        # below, and `find` takes the first match. Anchoring on `let ranked` keeps this pinned to the
        # function it names, which is the failure that first put this entry in the file.
        'find': "    let ranked: Vec<(MemoryId, f64)> = corpus\n        .vectors\n        .nearest(query, corpus.size())",
        'replace': "    let ranked: Vec<(MemoryId, f64)> = corpus\n        .vectors\n        .nearest_uncentred(query, corpus.size())",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # The walk's whole purpose in one line. Without the decayed share surviving into the score,
        # traversal returns nothing a larger window would not return sooner and more cheaply -- and
        # it drops exactly the memory it exists for: the one relevant because of its relation rather
        # than its wording.
        # Re-pointed 2026-08-19: validity now resolves before the ranking, so these live in the
        # candidate fold rather than in the admission loop.
        'name': "recall: rank on content similarity alone",
        'file': "src/store.rs",
        'find': "                traversal::combine_with(\n                    f64::from(candidate.similarity),\n                    carried,\n                    stages.inherited_weight,\n                ),",
        'replace': "                f64::from(candidate.similarity),",
        'tests': ["recall", "cross_store", "mcp"],
        'expect': "caught",
    },
    {
        # `result-count-follows-the-limit-not-the-query`. The limit is a count since 2026-10-02 -- this
        # entry replaced one that pinned the largest-gap cut, which no longer runs in what ships -- so
        # admitting one past it returns more than the caller asked for.
        'name': "recall: admit one more than the limit",
        'file': "src/store.rs",
        'find': "        if hits.len() >= question.limit {",
        'replace': "        if hits.len() > question.limit {",
        'tests': ["recall", "approval"],
        'expect': "caught",
    },
    {
        # `response-shape-is-unpublished`: output schemas and structured content belong to the
        # 2025-06-18 revision, so a client on the older one gets neither.
        'name': "mcp: publish output schemas to a client on the older protocol",
        'file': "src/mcp.rs",
        'find': '    protocol != "2024-11-05"',
        'replace': "    true",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "mcp: answer every client with the newest protocol",
        'file': "src/mcp.rs",
        'find': "        .and_then(|asked| PROTOCOL_VERSIONS.iter().find(|known| **known == asked))",
        'replace': "        .and_then(|_| None::<&&str>)",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The recall half of `a-scope-in-a-response-is-not-the-spelling-the-schema-publishes`,
        # found by the response-shape survey.
        'name': "mcp: serialise a reference's scope the derived way again",
        'file': "src/mcp.rs",
        'find': '        seq.serialize_element(&json!({\n            "scope": WireScope(&reference.scope),',
        'replace': '        seq.serialize_element(&json!({\n            "scope": reference.scope,',
        'tests': ["sets", "mcp"],
        'expect': "caught",
    },
    {
        # The client refused the whole tool list when an output schema's top level lacked this.
        'name': "output schema: drop the top-level object type from hoard_sets",
        'file': "src/output_schema.rs",
        'find': '    json!({\n        "type": "object",\n        "anyOf": [\n            action(',
        'replace': '    json!({\n        "anyOf": [\n            action(',
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # `the-missing-query-refusal-gives-no-example`. The refusal's example is the caller's call
        # with the query added; without that, a weak reader resent the call still missing it.
        'name': "recall: refuse a missing query without the call to make",
        'file': "src/mcp.rs",
        'find': '                example.insert("query".to_owned(), json!("<what you are looking for>"));',
        'replace': "",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # `recall-limit-is-unbounded`: the limit has no ceiling by decision, and a floor of one,
        # because an empty list from a limit of zero reads as *nothing matched*.
        'name': "recall: accept a limit of zero again",
        'file': "src/mcp.rs",
        'find': "                .filter(|&n| n >= 1)",
        'replace': "                .filter(|&n| n >= 0)",
        'tests': ["mcp", "approval"],
        'expect': "caught",
    },
    {
        # What the caller already holds is removed from the results and from nothing else. Returning
        # it anyway spends context on what the session already knows, and makes a second call unable
        # to mean anything but the first.
        # Re-pointed 2026-08-19: validity now resolves before the ranking, so these live in the
        # candidate fold rather than in the admission loop.
        'name': "recall: return what the caller said it already holds",
        'file': "src/store.rs",
        'find': "        if question.already_known.contains(&identity.id) {",
        'replace': "        if false {",
        'tests': ["recall", "mcp", "approval"],
        'expect': "caught",
    },
    {
        # Suppression must happen before the cut, or a memory removed after the fact leaves a slot
        # spent on nothing and paging returns less each time.
        # Re-pointed 2026-08-19: validity now resolves before the ranking, so these live in the
        # candidate fold rather than in the admission loop.
        'name': "recall: suppress the already-known after the cut instead of before it",
        'file': "src/store.rs",
        'find': "        if question.already_known.contains(&identity.id) {\n            continue;\n        }",
        'replace': "        if hits.len() > question.limit {\n            continue;\n        }",
        'tests': ["recall", "mcp"],
        'expect': "caught",
    },
    {
        # A scoped recall may not walk out of its scope, and the case that hides is the bridge: two
        # in-scope memories citing the same memory in a set this search never looked at. Score would
        # flow between them along a path nothing in the answer could account for.
        # Re-pointed 2026-08-18 when the population moved onto `Neighbourhood`.
        'name': "recall: walk through memories outside the searched sets",
        'file': "src/store.rs",
        'find': "            if neighbourhood.holds(source) && neighbourhood.holds(target) {",
        'replace': "            if true {",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # The store's own filter on which edges the walk may cross. Distinct from the mutation on
        # `is_traversable` itself: this one leaves the rule intact and stops consulting it.
        # Re-pointed 2026-08-18: the label test moved into `Store::edges`, which now splits the
        # rows two ways. Admitting every label to the walk also stops the `replaced` list being
        # built, so the walk both crosses supersession and stands on superseded memories -- one
        # mutation, two symptoms, and the tests object to both.
        'name': "recall: read supersession edges into the walk",
        'file': "src/store.rs",
        'find': '            if traversal::is_traversable(&label) {',
        'replace': "            if true {",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # The inherited score reaches the caller or the ordering is unexplainable: a memory arriving
        # above one with a better similarity, and nothing in the payload accounting for it.
        'name': "view: report every memory as unreached by the walk",
        'file': "src/mcp.rs",
        'find': "                    inherited: hit.inherited,\n                    supersession_cycle: hit.in_cycle,",
        'replace': "                    inherited: 0.0,\n                    supersession_cycle: hit.in_cycle,",
        'tests': ["mcp", "approval"],
        'expect': "caught",
    },
    {
        # Distance is the only thing that may reduce an inherited score, and it is the only weight
        # the corpus could speak to. A walk that does not decay makes a memory three hops out worth
        # as much as one the query matched.
        'name': "traversal: carry the score without decaying it",
        'file': "src/traversal.rs",
        'find': "            let carried = score * walk.decay;",
        'replace': "            let carried = *score;",
        'tests': ["traversal", "eval"],
        'expect': "caught",
    },
    {
        # The bound is on expansion, never on inclusion. Capping what is admitted would discard good
        # results to solve a problem they did not cause -- reaching a heavily-cited memory is cheap,
        # and expanding *through* it is what branches.
        # Re-pointed 2026-08-18 when the frontier became a map deduplicated into `ordered`.
        'name': "traversal: bound what the walk admits, not what it expands through",
        'file': "src/traversal.rs",
        'find': "        ordered.truncate(walk.frontier);",
        'replace': "        ordered.truncate(walk.frontier);\n        inherited.retain(|id, _| ordered.iter().any(|(other, _)| other == id));",
        'tests': ["traversal"],
        'expect': "caught",
    },
    {
        # A memory reached by several paths keeps the best, never the sum. Summing would make a
        # dense neighbourhood accumulate weight by being dense, which is in-degree entering the
        # ranking by the back door -- and the sign of an in-degree term is label-dependent and
        # deliberately unresolved.
        'name': "traversal: add up every path to a memory instead of keeping the best",
        'file': "src/traversal.rs",
        'find': "                if carried > *entry {\n                    *entry = carried;",
        'replace': "                if carried > *entry {\n                    *entry = if entry.is_finite() { *entry + carried } else { carried };",
        'tests': ["traversal"],
        'expect': "caught",
    },
    {
        # Cutting at the first gap makes recall a coin flip on whether the query happened to have
        # exactly one answer: the largest gap in a ranking is very often the one just below the best
        # match.
        'name': "traversal: allow the cut to fall directly below the top hit",
        'file': "src/traversal.rs",
        'find': "    for index in 2..limit {",
        'replace': "    for index in 1..limit {",
        'tests': ["traversal", "eval"],
        'expect': "caught",
    },
    {
        # The window is a ceiling, not a suggestion. Looking past it for a better cliff would return
        # more than the caller said it would accept.
        'name': "traversal: look past the window for a bigger drop",
        'file': "src/traversal.rs",
        'find': "    let limit = window.min(scores.len());",
        'replace': "    let limit = scores.len();",
        'tests': ["traversal", "recall"],
        'expect': "caught",
    },
    {
        # The two scores are blended. A maximum collapses a reached memory that also matches well
        # into one that barely matches; it also has the better single cell, which is what makes the
        # stability argument the whole reason this is not a maximum.
        'name': "traversal: take the larger of the two scores instead of blending them",
        'file': "src/traversal.rs",
        'find': "    (1.0 - weight) * own + weight * inherited",
        'replace': "    own.max(inherited)",
        'tests': ["traversal", "eval", "recall"],
        'expect': "caught",
    },
    {
        # A walk origin never inherits. Expansion is symmetric and the walk takes three hops, so a
        # path out of a seed and back returns 0.81 of its own score -- which `sum` then adds to the
        # seed's own similarity. That promotes a memory for having any edge at all, independently of
        # what the edge points at, which is in-degree entering the ranking by the back door.
        'name': "traversal: let a walk origin inherit from a path back to itself",
        'file': "src/traversal.rs",
        'find': "                if origins.contains(neighbour) {\n                    continue;\n                }",
        'replace': "",
        'tests': ["traversal", "recall"],
        'expect': "caught",
    },
    {
        # Seeds are resolved to distinct tips first. Without that, a supersession chain -- whose
        # members resemble each other by construction -- spends most of a five-seed budget on
        # memories that collapse to one, and the walk explores far less than its parameters claim.
        'name': "recall: seed the walk from unresolved ids",
        'file': "src/store.rs",
        'find': "        let tip = neighbourhood.tip_of(*at);",
        'replace': "        let tip = *at;",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # A link *to* a superseded memory belongs to its tip. Leaving it where it was authored makes
        # the dead memory a conduit for inherited score while the live one is short of routes, since
        # the only path between the two is the `SUPERSEDES` edge the walk refuses to cross.
        # Re-anchored 2026-09-24 onto the target alone, when step 3 of
        # out-links-stay-with-the-record-that-wrote-them stopped moving a superseded *source*.
        'name': "recall: leave the walk standing on superseded memories",
        'file': "src/store.rs",
        'find': "            let target = neighbourhood.tip_of(target);\n            if neighbourhood.holds(source)",
        'replace': "            if neighbourhood.holds(source)",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # The walk carries score to tips, so the ranking has to look it up by the identity it is
        # ranking rather than by whichever id matched. Recorded as *surviving* while the old
        # admission loop made the case hard to construct; resolving before the ranking made it
        # ordinary, and it is now a test rather than a paragraph of apology.
        # Repaired 2026-08-26: `identity` became `Located` -- an id plus which searched store it is
        # in -- when recall identity was partitioned by set, so `unwrap_or(identity)` stopped
        # type-checking against an `Option<MemoryId>`. Rebuilding a `Located` around the matched id
        # keeps the mutation's meaning: look the carried score up by the memory that *matched*
        # rather than by the one being ranked, which is what the fold was written to stop.
        'name': "recall: look up the inherited score by the id that matched",
        'file': "src/store.rs",
        'find': "                .get(&identity)",
        'replace': "                .get(&candidate.matched.map_or(identity, |id| Located { store: identity.store, id }))",
        'tests': ["recall", "approval"],
        'expect': "caught",
    },
    {
        # Fusion that ignores the walk is an expensive way to return the content ranking.
        'name': "eval: fuse the rankings without the walk's contribution",
        'file': "src/eval.rs",
        'find': "            let walked = reached_at.get(id).copied().unwrap_or(unreached);",
        'replace': "            let walked = unreached;",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # The top gap is the drop below the *leader*, which needs the scores sorted. Taken from the
        # order they arrive in it would report the drop between two arbitrary neighbours, and the
        # figure would still look plausible -- it is a small positive number either way.
        'name': "eval: measure the top gap without ordering the scores",
        'file': "src/eval.rs",
        'find': "        let mut ordered: Vec<f64> = scores.to_vec();\n        ordered.sort_by(|a, b| b.total_cmp(a));",
        'replace': "        let ordered: Vec<f64> = scores.to_vec();",
        'tests': ["eval"],
        'expect': "caught",
    },
    {
        # The weight is the decision, and the direction matters: three quarters on the inherited term
        # is the setting measured to do *worse than not walking at all*, so a mutation that flips it
        # is not a near miss.
        'name': "traversal: weight the inherited share at three quarters",
        'file': "src/traversal.rs",
        'find': "pub const INHERITED_WEIGHT: f64 = 0.25;",
        'replace': "pub const INHERITED_WEIGHT: f64 = 0.75;",
        'tests': ["traversal", "eval", "recall"],
        'expect': "caught",
    },
    {
        # A cycle is the one data error resolution can detect, and both callers used to discard it.
        # Folding it back into `Current` is the defect exactly as it stood.
        'name': "recall: treat a supersession cycle as a current memory",
        'file': "src/store.rs",
        'find': "            Some(Standing::Cycle) => Ranked::InCycle,",
        'replace': "            Some(Standing::Cycle) => Ranked::Itself,",
        'tests': ["recall", "mcp"],
        'expect': "caught",
    },
    {
        # A memory known to be superseded by something this machine does not hold must be withheld,
        # not passed off as current: it is known to be stale and its replacement is unreadable.
        'name': "recall: return a memory superseded elsewhere without flagging it",
        'file': "src/store.rs",
        'find': "                None => Ranked::Unreplaced {\n                    beyond: *unresolved,\n                    forgotten: *forgotten,\n                },",
        'replace': "                None => Ranked::Itself,",
        'tests': ["recall", "cross_store"],
        'expect': "caught",
    },
    {
        # The conjunction is the fix: framing present *and* the parameter it names absent. Dropping
        # the second half restores the disjunction, which flagged any memory quoting the markup --
        # including a memory about this very defect, which this project writes repeatedly.
        'name': "Transmission damage: flag on the framing alone, ignoring what arrived",
        'file': "src/mcp.rs",
        'find': "            absent(field)\n                && framing",
        'replace': "            true\n                && framing",
        'tests': ["mcp", "approval"],
        'expect': "caught",
    },
    {
        # And the other half: detecting nothing at all, which is where this started.
        'name': "Transmission damage: never detect a swallowed parameter",
        'file': "src/mcp.rs",
        'find': "        .map(|(field, _)| *field)\n        .collect()",
        'replace': "        .map(|(field, _)| *field)\n        .take(0)\n        .collect()",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The escape has to be the precondition it was recorded as: without decoding, a memory whose
        # subject is markup emits the markup and is refused, which is the failure the ordering exists
        # to prevent.
        'name': "escape: store the entities as written instead of decoding them",
        'file': "src/mcp.rs",
        'find': "    MemoryText::new(decode_entities(text.as_data()))",
        'replace': "    MemoryText::new(text.as_data().to_owned())",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The refusal is only worth anything if it names *every* argument that failed to arrive.
        # Stopping at the first restores serde's one real limitation -- the reason this was checked
        # ahead of the deserialiser rather than translated after it.
        'name': "missing argument: name only the first argument that did not arrive",
        'file': "src/mcp.rs",
        'find': "            .filter(|name| !supplied(arguments, name))\n            .map(|name| (*name).to_owned())\n            .collect();",
        'replace': "            .filter(|name| !supplied(arguments, name))\n            .map(|name| (*name).to_owned())\n            .take(1)\n            .collect();",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # And the guard itself: never refusing sends the call on to the deserialiser, which is the
        # defect. Proves the new tests fail on the serde message rather than merely on some message.
        'name': "missing argument: let every call through to the deserialiser",
        'file': "src/mcp.rs",
        'find': "    if absent.is_empty() {\n        return None;\n    }",
        'replace': "    if true {\n        return None;\n    }",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The whole defect in one line: without the cut, each store's own `limit` stands and the
        # response holds `limit` times the number of loaded sets.
        'name': "keyword recall: let every store contribute its own full limit",
        'file': "src/store.rs",
        'find': '    while hits.len() < limit {\n        let Some(best) = heads',
        'replace': '    while hits.len() < limit * stores.len() {\n        let Some(best) = heads',
        'tests': ["cross_store", "mcp"],
        'expect': "caught",
    },
    {
        # The half that is easier to miss: concatenation is not ranking. Ordering by id alone still
        # looks sorted and still answers deterministically -- it is simply not best-first.
        'name': "keyword recall: order the union by id rather than by match count",
        'file': "src/store.rs",
        'find': '                right\n                    .matched_keywords\n                    .cmp(&left.matched_keywords)\n                    .then_with(|| left.memory.id.cmp(&right.memory.id))',
        'replace': '                left.memory.id.cmp(&right.memory.id)',
        'tests': ["cross_store", "mcp"],
        'expect': "caught",
    },
    {
        # And the tie-break, which only a two-store test can see: dropping it leaves equal match
        # counts in whatever order the stores were handed over.
        'name': "keyword recall: leave an equal match count in store order",
        'file': "src/store.rs",
        'find': "            .then_with(|| left.memory.id.cmp(&right.memory.id))",
        'replace': "",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # Crockford's alphabet is in value order, which is the only reason the text form sorts like
        # the value. A rotated alphabet still round-trips, so nothing but the ordering test notices.
        'name': "ids: encode with an alphabet that is not in value order",
        'file': "src/memory/id.rs",
        'find': 'const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";',
        'replace': 'const CROCKFORD: &[u8; 32] = b"Z0123456789ABCDEFGHJKMNPQRSTVWXY";',
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # `generated-ids-can-sort-out-of-creation-order`. A candidate equal to the last id issued is a
        # duplicate, so `>=` would hand the same id out twice.
        'name': "ids: keep a candidate equal to the last id issued",
        'file': "src/memory/id.rs",
        'find': "    if candidate > last {\n        return candidate;",
        'replace': "    if candidate >= last {\n        return candidate;",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        'name': "ids: step past the last id with a plain increment",
        'file': "src/memory/id.rs",
        'find': "    let free = ((last >> 80) << 74)",
        'replace': "    return last + 1;\n    let free = ((last >> 80) << 74)",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        'name': "ids: let generation take the crate's id unchecked",
        'file': "src/memory/id.rs",
        'find': "    *last = step_past(candidate, *last);",
        'replace': "    *last = candidate;",
        'tests': ["memory_model"],
        'expect': "survives",
        'reason': """The crate hands back an out-of-order id only when a thread is pre-empted between reading
the clock and taking its lock across a second boundary, or the clock steps back across one. No test
can arrange either; `ids_sort_into_creation_order` has caught it exactly once, in a full-suite run
on 2026-09-22. The decision itself is pinned through `MemoryId::or_next_after` with ids chosen by hand; this
line only routes `generate` through it, which is visible in the source.""",
    },
    {
        # The compatibility window: a set travels between machines that upgrade at different times,
        # so dropping the previous spelling strands every file and store already written.
        'name': "ids: forget how to read the previous hex spelling",
        'file': "src/memory/id.rs",
        'find': "        Uuid::parse_str(value)\n            .map(Self)",
        'replace': '        Uuid::parse_str("")\n            .map(Self)',
        'tests': ["memory_model", "store"],
        'expect': "caught",
    },
    {
        # Crockford over plain base32 was chosen *for* the confusable mappings. Without them a
        # fragment transcribed wrongly stops resolving, which is the recovery case.
        'name': "ids: stop resolving Crockford's confusable characters",
        'file': "src/memory/id.rs",
        'find': "        b'O' => Some(0),",
        'replace': "        b'O' => None,",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # Without the migration an older store opens healthy and answers nothing: every id in it is
        # spelled the old way and no lookup matches.
        'name': "ids: never re-encode a store written in the previous spelling",
        'file': "src/store.rs",
        'find': "        if pending.is_empty() {\n            return Ok(());\n        }",
        'replace': "        if true {\n            return Ok(());\n        }",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The regression that would undo the point: a substring search quietly becoming a prefix one
        # puts the caller back on the single worst cut position in the string.
        'name': "lookup: search by prefix instead of substring",
        'file': "src/store.rs",
        'find': 'format!("%{needle}%")',
        'replace': 'format!("{needle}%")',
        'tests': ["other_paths", "mcp"],
        'expect': "caught",
    },
    {
        # An ambiguous result that does not say which end of an id to quote leaves the caller to
        # repeat the mistake -- the message is the mitigation, not a decoration.
        'name': "lookup: stop remarking when a fragment is ambiguous",
        'file': "src/mcp.rs",
        'find': "        let ambiguous = params.fragment.is_some() && hits.len() > 1;",
        'replace': "        let ambiguous = false;",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Fragment normalisation is also what keeps `%` and `_` out of the LIKE pattern.
        'name': "lookup: let a fragment carry characters that are not id characters",
        'file': "src/memory/id.rs",
        'find': "            .map(|ch| crockford_digit(ch).map(|digit| char::from(CROCKFORD[usize::from(digit)])))",
        'replace': "            .map(Some)",
        'tests': ["other_paths"],
        'expect': "caught",
    },
    {
        # The defect itself: `export` reporting records written under the name `status` uses for
        # live memories. The two disagreed by the tombstone count, at the export gate.
        'name': "counts: report records written under the name `memories`",
        'file': "src/mcp.rs",
        'find': '            "memories": live,',
        'replace': '            "memories": rendered.written,',
        'tests': ["sets", "approval"],
        'expect': "caught",
    },
    {
        # And the other half: a `forgotten` wired to the wrong source still reports a number, and a
        # number with a plausible value is exactly what nobody re-checks.
        'name': "counts: wire `forgotten` to the live count",
        'file': "src/store.rs",
        'find': '            .query_row("SELECT COUNT(*) FROM tombstones", [], |row| row.get(0))?;',
        'replace': '            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;',
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The whole guard: without it the reserved directory nests inside itself and the call
        # reports success with the doubled path.
        'name': "reserved directory: nest inside it instead of refusing",
        'file': "src/mcp.rs",
        'find': "    if directory.file_name()? != std::ffi::OsStr::new(RESERVED_DIRECTORY) {",
        'replace': "    if true {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `file_name` is what makes a trailing separator harmless. Comparing the whole path instead
        # still refuses the common spelling and quietly lets `.../scatterhoard_mem/` through.
        'name': "reserved directory: compare the whole path rather than its last segment",
        'file': "src/mcp.rs",
        'find': "    if directory.file_name()? != std::ffi::OsStr::new(RESERVED_DIRECTORY) {",
        'replace': "    if !directory.to_string_lossy().ends_with(RESERVED_DIRECTORY) {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The conflation itself: reporting the run/coverage question as if it were the other one.
        # With `searched` in place of `complete`, a partial union claims nothing was searched while
        # returning a ranking -- which is what the defect was.
        'name': "coverage: report whether the search ran in place of whether it covered everything",
        'file': "src/mcp.rs",
        'find': '            "semantic_search_available": coverage.complete,',
        'replace': '            "semantic_search_available": coverage.searched,',
        'tests': ["sets", "approval"],
        'expect': "caught",
    },
    {
        # Naming the skipped set is the half that makes it actionable; a count cannot say which
        # store to act on, and the remedy differs by reason.
        'name': "coverage: count the skipped sets instead of naming them",
        'file': "src/store.rs",
        'find': "                reason: store.why_unsearchable()?,",
        'replace': '                reason: String::new(),',
        'tests': ["cross_store", "sets"],
        'expect': "caught",
    },
    {
        # The keyword path runs no semantic search, so it must report capability rather than a
        # guess. Reporting `searched` there is the defect SETUP item 7 exists for.
        'name': "coverage: let the keyword path claim it searched by meaning",
        'file': "src/mcp.rs",
        'find': "                coverage.complete = coverage.skipped.is_empty();",
        'replace': "                coverage.complete = coverage.searched;",
        'tests': ["approval", "mcp"],
        'expect': "caught",
    },
    {
        # The strict invariant: a read that rebuilds is the defect, whatever its trigger converges to.
        'name': "model: let a read rebuild vectors of unknown provenance",
        'file': "src/store.rs",
        'find': "            ModelState::Mismatched { .. } | ModelState::ProvenanceUnknown { .. }",
        'replace': "            ModelState::Mismatched { .. }",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # No vectors, no model. Without the release a store claims a model it holds no evidence for,
        # which is what made import on a modelless server report a mismatch against nothing.
        'name': "model: keep the recorded model after the last vector goes",
        'file': "src/store.rs",
        'find': "        if self.vector_count()? == 0 {",
        'replace': "        if false {",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # A reset displaces nothing. Reporting the same identity as displaced describes a change
        # that did not happen, and would accumulate into the alternation count that detects two
        # servers fighting -- so resetting twice would look like a conflict.
        'name': "model: report a reset as having displaced the model it re-applied",
        'file': "src/store.rs",
        'find': "                (held != model).then_some(held)",
        'replace': "                Some(held)",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # A tombstone is a resolved link target. Dropping it reports a supersession chain whose
        # middle was forgotten as broken -- a correct link called a mistake, which is the direction
        # that teaches a caller to distrust the report.
        'name': "links: stop counting a tombstone as a record the store holds",
        'file': "src/store.rs",
        'find': "                \"SELECT 1 FROM memories WHERE id = ?1\n                 UNION ALL\n                 SELECT 1 FROM tombstones WHERE id = ?1\",",
        'replace': "                \"SELECT 1 FROM memories WHERE id = ?1\",",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The check that did not run must not look like one that passed. This is the shape
        # `prohibited-data-scanned-conflates-skipped-with-unbuilt` was filed for, one layer over.
        'name': "links: treat a set this machine does not hold as resolved",
        'file': "src/mcp.rs",
        'find': "                None => LinkState::SetNotLoaded,",
        'replace': "                None => continue,",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Reporting every link, resolved or not, is as useless as reporting none: the caller cannot
        # tell which one to look at.
        'name': "links: report resolved targets alongside unresolved ones",
        'file': "src/mcp.rs",
        'find': "                    if held {\n                        continue;",
        'replace': "                    if false {\n                        continue;",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # `two-import-paths-disagree-about-supersession-cycles`. The refusal on `insert` is right for
        # a caller authoring one edge and wrong for a file, where it lands on whichever record the
        # cycle closed on and aborts the import with the store already partly written.
        'name': "import: refuse a cycle per record instead of accepting the file",
        'file': "src/mcp.rs",
        'find': "                                .accept_from_file(memory)",
        'replace': "                                .insert(memory)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Accepting a cycle silently would trade an abort for a stranger failure: resolution reports
        # such a memory as `supersession_cycle` at recall, with nothing saying where it came from.
        'name': "import: accept a cycle without reporting it",
        'file': "src/mcp.rs",
        'find': "                Ok(Resolution::Cycle) => cycles.push(id.to_string()),",
        'replace': "                Ok(Resolution::Cycle) => {}",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Luhn is what makes the card rule usable rather than constantly wrong: memory sets are full
        # of long digit runs, and roughly nine in ten fail the checksum.
        'name': "prohibited: accept any long digit run as a card number",
        'file': "src/prohibited.rs",
        'find': "        if (13..=19).contains(&digits.len()) && luhn(&digits) {",
        'replace': "        if (13..=19).contains(&digits.len()) {",
        'tests': ["prohibited"],
        'expect': "caught",
    },
    {
        # Without the minimum run, a prefix alone matches -- so a memory naming which prefix a vendor
        # uses becomes unstorable, which is the vocabulary trap arriving by another route.
        'name': "prohibited: treat a bare vendor prefix as a key",
        'file': "src/prohibited.rs",
        'find': "            if starts_a_word && run >= *minimum {",
        'replace': "            if starts_a_word {",
        'tests': ["prohibited"],
        'expect': "caught",
    },
    {
        # The write is where refusal belongs; dropping it puts the credential in the store.
        'name': "prohibited: store a credential instead of refusing the write",
        'file': "src/mcp.rs",
        'find': "            (\"context\", context.as_ref().map_or(\"\", MemoryText::as_data)),\n        ]) {\n            return Err(refusal);\n        }",
        'replace': "            (\"context\", context.as_ref().map_or(\"\", MemoryText::as_data)),\n        ]) {\n            let _ = refusal;\n        }",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # And the export must *not* refuse: a set can hold imported memories the caller cannot fix,
        # and a filter that gains a rule would otherwise close a set that exported fine yesterday.
        'name': "prohibited: refuse the export instead of warning",
        'file': "src/mcp.rs",
        'find': "        if !prohibited_found.is_empty() {\n            result[\"prohibited_data_found\"]",
        'replace': "        if !prohibited_found.is_empty() {\n            return Err(\"refused\".to_owned());\n        }\n        if !prohibited_found.is_empty() {\n            result[\"prohibited_data_found\"]",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Without the migration an older store keeps the cascading foreign key, and every amendment
        # silently destroys the base it should have preserved.
        'name': "bases migration: leave an older store cascading",
        'file': "src/store.rs",
        'find': "        if cascades {\n            self.connection.execute_batch(\n                \"DROP TABLE bases;",
        'replace': "        if false {\n            self.connection.execute_batch(\n                \"DROP TABLE bases;",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The permissive direction is the worse failure: the table is dropped on *every* open, so
        # every record reads as having no ancestor at every import, silently and for ever.
        'name': "bases migration: fire on every store, not only a faulty one",
        'file': "src/store.rs",
        'find': "            .prepare(\"SELECT COUNT(*) FROM pragma_foreign_key_list('bases')\")?\n            .query_row([], |row| row.get::<_, i64>(0))?\n            > 0;",
        'replace': "            .prepare(\"SELECT COUNT(*) FROM pragma_foreign_key_list('bases')\")?\n            .query_row([], |row| row.get::<_, i64>(0))?\n            >= 0;",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # `origin-cannot-be-corrected`. The handler half of the fix.
        'name': "origin: accept the amendment and drop it on the floor",
        'file': "src/mcp.rs",
        'find': "        if let Some(origin) = params.origin {\n            memory.origin = origin;",
        'replace': "        if let Some(origin) = params.origin {\n            let _ = origin;",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The schema half, which is the one that would go unnoticed: a validating client strips an
        # argument the schema does not declare, so the capability ships unreachable and every
        # handler-level test still passes.
        'name': "origin: amend it, but do not publish that you can",
        'file': "src/mcp.rs",
        'find': "                    \"origin\": {\n                        \"type\": \"string\",\n                        \"enum\": Origin::ALL.iter().map(serde_name).collect::<Vec<_>>(),\n                        \"description\": \"Corrects whose reasoning produced the content",
        'replace': "                    \"unpublished_origin\": {\n                        \"type\": \"string\",\n                        \"enum\": Origin::ALL.iter().map(serde_name).collect::<Vec<_>>(),\n                        \"description\": \"Corrects whose reasoning produced the content",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The reservation is the whole fix for `target-set-local-names-a-different-set`: without it
        # `Scope::Shared(SetName("local"))` is constructible again and the same word means two sets
        # in one call.
        'name': "set name: stop reserving `local`",
        'file': "src/memory/scope.rs",
        'find': "        if value.eq_ignore_ascii_case(\"local\") {",
        'replace': "        if false {",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # Case-folded on purpose: the confusion is about how the word reads, and `Local` collides
        # with `Scope::Local` for every human who meets it.
        'name': "set name: reserve `local` only in lowercase",
        'file': "src/memory/scope.rs",
        'find': "        if value.eq_ignore_ascii_case(\"local\") {",
        'replace': "        if value == \"local\" {",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # The whole fix for `a-set-can-suppress-another-sets-memory`. Without the guard, a cross-set
        # SUPERSEDES pushes a foreign id into `replaced`, `Neighbourhood::over` asks this store to
        # resolve a memory it does not hold, and the answer joins a validity map the whole recall
        # shares -- so a rival set removes a memory it never held.
        'name': "edges: let a cross-set SUPERSEDES mark its target replaced",
        'file': "src/store.rs",
        'find': "                LinkLabel::Supersedes if target_set.is_none() => edges.replaced.push(target),",
        'replace': "                LinkLabel::Supersedes => edges.replaced.push(target),",
        'tests': ["cross_store"],
        'expect': "survives",
        'reason': """Belt and braces with the `incoming` guard below, and on the current code the braces alone
hold. Reverting only this one leaves the foreign id entering `replaced`, so `Neighbourhood::over`
does ask the wrong store to resolve a memory it does not hold -- but `incoming` then matches
nothing, because the rival's row names another set, and `links_of` cannot help either: a link row
requires its source memory to exist (`REFERENCES memories(id)`), so a store holds no links for an
id it does not hold. The chain resolves Current and nothing is suppressed. No test can separate
them without removing the other guard, which would be testing a configuration we do not ship.

Kept anyway, for two reasons. It states the rule at the layer the rule is about -- an edge only
speaks about a target in its own set -- where `incoming`'s condition reads as a lookup detail. And
it is the guard that still holds if `incoming` is ever widened to answer a legitimate cross-set
question, which is a plausible future: reporting the edge, per
`documents/design/cross-set-references.md`, needs some lookup to find it.""",
    },
    {
        # The other half of the same rule, and the one with no test of its own until now: a link row
        # whose target names another set must not answer a question about an id held here, or an id
        # collision between two loaded sets bridges them.
        'name': "incoming: match edges regardless of which set they point into",
        'file': "src/store.rs",
        'find': "            \"SELECT source, label FROM links WHERE target = ?1 AND target_set IS NULL \\\n             ORDER BY source\",",
        'replace': "            \"SELECT source, label FROM links WHERE target = ?1 ORDER BY source\",",
        'tests': ["cross_store", "recall"],
        'expect': "caught",
    },
    {
        # `SUPERSEDED_BY` is exempt on purpose -- a memory conceding its own standing rather than
        # reaching for someone else's. Folding it into the refusal takes away the only form that
        # survives the replacement's set being unavailable.
        'name': "write refusal: refuse a cross-set SUPERSEDED_BY as well",
        'file': "src/mcp.rs",
        'find': "                    LinkLabel::Supersedes | LinkLabel::ExceptionTo | LinkLabel::Contradicts",
        'replace': "                    LinkLabel::Supersedes | LinkLabel::ExceptionTo | LinkLabel::Contradicts | LinkLabel::SupersededBy",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Returned-and-flagged rather than withheld. Dropping it again is silent: the memory simply
        # is not there, which is the failure mode the change was made to remove.
        'name': "recall: withhold a memory superseded into an unloaded set",
        'file': "src/store.rs",
        'find': "            Ranked::Unreplaced { beyond, forgotten } => (at, None, false, beyond, forgotten),",
        'replace': "            Ranked::Unreplaced { .. } => continue,",
        'tests': ["cross_store", "recall"],
        'expect': "caught",
    },
    {
        # `SUPERSEDED_BY` is not a dispute: aimed at our memory it concedes the claimant's own
        # standing. Counting it would report every concession as an attack.
        'name': "disputes: count SUPERSEDED_BY as a claim against us",
        'file': "src/store.rs",
        'find': "const DISPUTING_LABELS: [LinkLabel; 3] = [\n    LinkLabel::Supersedes,",
        'replace': "const DISPUTING_LABELS: [LinkLabel; 4] = [\n    LinkLabel::SupersededBy,\n    LinkLabel::Supersedes,",
        'tests': ["cross_store", "mcp"],
        'expect': "caught",
    },
    {
        # A claim from our own set is not a claim -- it is a same-set link spelled the long way,
        # which `a-same-set-link-spelled-with-its-set-name-is-inert` covers. Reporting it would tell
        # a caller their own set was attacking them.
        'name': "disputes: ask the memory's own store for claims against it",
        'file': "src/store.rs",
        'find': "    let mut claims = Vec::new();\n    for store in stores {\n        if &store.scope == scope {\n            continue;\n        }",
        'replace': "    let mut claims = Vec::new();\n    for store in stores {\n        if false {\n            continue;\n        }",
        'tests': ["cross_store"],
        'expect': "survives",
        'reason': """Caught when it was written, and stopped being catchable a few hours later when
`a-same-set-link-spelled-with-its-set-name-is-inert` was fixed. That fix canonicalises a link naming
its own set into the compact form at the write, so no row can any longer carry a `target_set` equal
to the set holding it -- and this guard exists to ignore exactly such a row. The condition is now
unreachable rather than untested, which is the outcome STANDARDS.md prefers: an error made
impossible to express rather than detected at run time.

Kept rather than deleted, and the reason is the one this file exists to record. The guard states the
rule at the layer the rule belongs to -- a claim is defined by crossing a set boundary -- so it holds
if canonicalisation is ever relaxed, moved, or bypassed by a new write path. Deleting it would make
the reporting silently depend on a canonicalisation two modules away, with nothing saying so.

Do not write a test for it. Reaching the condition means planting a row that the write path is
designed to make unwritable, so any such test would be asserting against a fixture no supported code
path can produce."""
    },
    {
        # The field is `skip_serializing_if`, so a wrong predicate hides every dispute from every
        # caller while the store-level tests stay green. This is the mutation that proves the
        # reporting reaches the wire at all.
        'name': "disputes: never serialise the claim list",
        'file': "src/mcp.rs",
        'find': "fn no_claims(claims: &&[DisputedBy]) -> bool {\n    claims.is_empty()",
        'replace': "fn no_claims(claims: &&[DisputedBy]) -> bool {\n    let _ = claims;\n    true",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # Reporting must not reach into the claimant's content. If a claim could carry it, disputing
        # a memory would buy a voice in every recall that memory matches.
        'name': "disputes: match any target_set rather than the hit's own set",
        'file': "src/store.rs",
        'find': "            \"SELECT source, label FROM links WHERE target = ?1 AND target_set = ?2 ORDER BY source\",",
        'replace': "            \"SELECT source, label FROM links WHERE target = ?1 ORDER BY source\",",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # The data half of `a-same-set-link-spelled-with-its-set-name-is-inert`. A store imported
        # before canonicalisation holds the long spelling, and those rows are inert.
        'name': "links: skip compacting a store's own-set link rows",
        'file': "src/store.rs",
        'find': "        let Some(own) = self.scope.exports_to() else {\n            return Ok(());\n        };",
        'replace': "        let Some(own) = None::<&SetName> else {\n            return Ok(());\n        };",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The wrong question, and the one the code used to ask: `does this carry a set name` rather
        # than `is it in a different set`. Catches an ordinary same-set correction spelled the long
        # way, at the write door and at the write.
        'name': "links: treat any named target as foreign",
        'file': "src/memory/link.rs",
        'find': "            Self::CrossSet { set, .. } => scope.exports_to() != Some(set),",
        'replace': "            Self::CrossSet { .. } => true,",
        'tests': ["mcp", "cross_store"],
        'expect': "caught",
    },
    {
        # The opposite error, and the worse one: treating every named target as local would strip
        # the set name off genuinely foreign links and resolve them against the wrong set.
        'name': "links: treat every named target as our own",
        'file': "src/memory/link.rs",
        'find': "            Self::CrossSet { set, .. } => scope.exports_to() != Some(set),",
        'replace': "            Self::CrossSet { .. } => false,",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # Canonicalisation at the one place every write passes through. Without it the two spellings
        # both reach the table and every lookup over it has to compare names.
        'name': "links: store the long spelling as written",
        'file': "src/store.rs",
        'find': "            } if link.target.is_foreign_to(scope, holder) => (",
        'replace': "            } if true => (",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # Naming only the repeat is what made the guard look absent: a line number finds the second
        # occurrence and cannot find the first, which is the one the caller compares against.
        'name': "memory file: report the duplicate id without naming it",
        'file': "src/memory_file.rs",
        'find': "        \"lines {first} and {line} both claim memory id {id}. A file may carry an id once: two \\",
        'replace': "        \"lines {first} and {line} repeat an id. A file may carry an id once: two \\",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # The structural half of `a-set-can-suppress-another-sets-memory`, and the half the first
        # repair missed. Filtering on `target_set` keyed on the link's spelling; a bare id bypassed
        # it. A store may only rule on memories it holds.
        'name': "neighbourhood: let a store resolve an id it does not hold",
        'file': "src/store.rs",
        'find': "                if !store.holds_record(replaced)? {\n                    continue;\n                }",
        'replace': "                if false {\n                    continue;\n                }",
        'tests': ["cross_store"],
        'expect': "survives",
        'reason': (
            "Subsumed by located identity on 2026-08-26, and kept deliberately. This was the "
            "structural half of `a-set-can-suppress-another-sets-memory`: a rival file writing "
            "`SUPERSEDES ours` as a bare id made its store resolve our id, and the answer went into "
            "a validity map keyed by id that the whole recall shared. That map is now keyed by store "
            "*and* id, so the rival's ruling lands under its own store's key -- a key nothing in the "
            "population answers to, since the population is each store's own live memories. Traced "
            "rather than assumed: `scored` yields only population members, `SUPERSEDES` is not "
            "walkable so no edge carries such a key into `tip_of`, and a bogus entry is therefore "
            "reachable from nothing. The line stays because it states the rule where the rule "
            "belongs -- a store may only speak about memories it holds -- and because it is the "
            "guard that still holds if the key is ever loosened. `a_rival_set_cannot_supersede_a_"
            "memory_it_does_not_hold_with_a_bare_id` continues to pass and now passes for two "
            "reasons rather than one."
        ),
    },
    {
        # Absorbing a file that declares *this* set is `import` or `restore`, and reading it as an
        # absorption would make the operation a second spelling of one of them -- with a safety copy
        # and a supersession pass nobody asked for.
        'name': "absorb: accept a file declaring the absorbing set itself",
        'file': "src/mcp.rs",
        'find': "        if same_set {",
        'replace': "        if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Collapsing two distinct memories onto one id is the one thing set-scoped ids make possible
        # and nothing else prevents. Treating a collision as an already-held record would write the
        # arriving one over a memory that says something else.
        'name': "absorb: treat a colliding id as a record already held",
        'file': "src/mcp.rs",
        'find': "                Some(_) => collisions.push(record.id().to_string()),",
        'replace': "                Some(_) => already_held += 1,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A superseded memory keeps its cross-set link because that is what it said when it was in
        # service. Superseding it again would author a second replacement for a claim already
        # replaced -- and would make a second absorption of one file do work rather than nothing.
        'name': "absorb: supersede memories that have already been superseded",
        'file': "src/mcp.rs",
        'find': '                .map_err(|error| format!("could not read this set\'s supersessions: {error}"))?\n            {\n                Resolution::Current => {',
        'replace': '                .map(|_| Resolution::Current)\n                .map_err(|error| format!("could not read this set\'s supersessions: {error}"))?\n            {\n                Resolution::Current => {',
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The whole of the change: reusing the original's id makes the supersession a *rewrite*
        # again, which is the receiver-side edit `absorb-as-supersession` exists to remove. One id
        # would once more hold two contents, with no ancestry to say so.
        'name': "absorb: reuse the original's id instead of minting one",
        'file': "src/mcp.rs",
        'find': "            replacement.id = MemoryId::generate();",
        'replace': "            replacement.id = original.id;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The collection step: which memories are candidates at all. Caught until 2026-09-28, when
        # the rewrite and the report each gained their own check on the absorbed set; see `reason`.
        'name': "absorb: collect every memory with any foreign link as a candidate",
        'file': "src/mcp.rs",
        'find': "                    && memory.links.iter().any(&names_absorbed)",
        'replace': "                    && memory.links.iter().any(|link| link.target.set().is_some())",
        'tests': ["sets"],
        'expect': "survives",
        'reason': "Since 2026-09-28 (`absorb-internalises-links-to-memories-that-did-not-arrive`) "
                  "every use of the candidates re-checks the set: `internalises` decides what is "
                  "superseded and the `not_internalised` filter what is reported. Widening the "
                  "collection only resolves the supersession of more memories, so the filter is "
                  "a cost bound rather than a guard, and no test can see it.",
    },
    {
        # An allow-list, not a wildcard, and the property survives the move from `import`'s argument
        # into the file's own header: a link into a set this file does not declare stays foreign.
        # Rewriting every foreign link would hand any file authority over any id it named.
        'name': "absorb: make every foreign link internal, not just the absorbed set's",
        'file': "src/mcp.rs",
        'find': "                if internalises(link) {",
        'replace': "                if link.target.set().is_some() {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The arriving records are written as their author wrote them. Stamping the absorbing set's
        # identity on them is the same plausible-looking mistake the import path has a mutation for,
        # and it would rewrite the authorship of everyone else's memories on a merge.
        'name': "absorb: re-attribute the arriving records to the absorbing set",
        'file': "src/mcp.rs",
        'find': "                    store.accept_from_file(memory).map_err(|error| {",
        'replace': "                    let mut memory = (**memory).clone();\n                    memory.created.by = author.clone();\n                    memory.modified.by = author.clone();\n                    store.accept_from_file(&memory).map_err(|error| {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The gate's forward half. Until 2026-08-31 this comparison was on the major alone, which
        # while every version begins `0.` refused nothing -- so an incompatible file was announced
        # by a malformed record partway through it rather than by a refusal naming two versions.
        'name': "format gate: accept a file from a newer version",
        'file': "src/memory_file.rs",
        'find': "        Some(unit) if unit > writes => Err(MemoryFileError::VersionTooNew {",
        'replace': "        Some(unit) if false && unit > writes => Err(MemoryFileError::VersionTooNew {",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # The floor. Dropping it makes `OLDEST_READABLE` decorative, which is the state the whole
        # constant exists to leave -- and the lever a format cut pulls would then pull nothing.
        'name': "format gate: read a file older than this build understands",
        'file': "src/memory_file.rs",
        'find': "        Some(unit) if unit < oldest => Err(MemoryFileError::VersionTooOld {",
        'replace': "        Some(unit) if false && unit < oldest => Err(MemoryFileError::VersionTooOld {",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # The minor is the compatibility unit only below 1.0. Counting it at every major would make
        # an ordinary post-1.0 minor bump -- a compatible addition, by semver -- refuse files this
        # build reads perfectly well.
        'name': "format gate: compare the minor at every major, not just below 1.0",
        'file': "src/memory_file.rs",
        'find': "    Some(if major == 0 { (0, minor) } else { (major, 0) })",
        'replace': "    Some((major, minor))",
        'tests': ["memory_file"],
        'expect': "survives",
        'reason': (
            "No file this project has written declares a major above 0, so the two rules agree on "
            "every input the suite can construct, and a test for the post-1.0 branch would have to "
            "assert behaviour on a version the format has never reached. Left as a recorded "
            "survivor rather than pinned by a contrived test: the branch states the rule where the "
            "rule belongs, and the day `1.0.0` is written is the day it becomes reachable and this "
            "entry should flip to `caught`. Compare `store: stop clearing links before rewriting`, "
            "which flipped exactly that way on 2026-08-30."
        ),
    },
    {
        # An unparseable version is read as newer, and the direction is the safe one: refusing.
        # Treating it as readable would let a corrupted header through to the record parser.
        'name': "format gate: treat an unparseable version as readable",
        'file': "src/memory_file.rs",
        'find': "        None => Err(MemoryFileError::VersionTooNew {\n            found: found.to_owned(),\n            writes: FORMAT_VERSION.to_owned(),\n        }),",
        'replace': "        None => Ok(()),",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # Filling an absence converges; displacing a held value oscillates, letting two servers restamp a
        # set's identity forever. Since `set-settled-at-first-write` held means held, records or not:
        # `create` no longer mints, so the empty-set exception had nothing left to repair.
        'name': 'set identity: displace an identity a set holds',
        'file': 'src/store.rs',
        'find': "             ON CONFLICT(key) DO NOTHING\",\n            params![id.to_string()],",
        'replace': "             ON CONFLICT(key) DO UPDATE SET value = excluded.value\",\n            params![id.to_string()],",
        'tests': ['store', 'concurrency'],
        'expect': "caught",
    },
    {
        # Minting for an existing set is the auto-mint hazard the migration rejected: two stores
        # holding one set would invent rival identities and fork it silently.
        'name': "set identity: mint a fresh identity over one already minted",
        'file': "src/store.rs",
        'find': "        if let Some(existing) = self.set_id()? {\n            return Ok(existing);\n        }",
        'replace': "",
        # `store` rather than `sets`: every write asks for a mint, so a second one is the ordinary
        # case, and the rule belongs to the layer that owns it.
        'tests': ["store"],
        'expect': "survives",
        'reason': "Since 2026-09-28 (`concurrent-first-writes-can-mint-two-identities`) this read "
                  "is a shortcut and not the guard: without it the call claims, the claim's "
                  "insert-if-absent leaves the held identity alone, and the held one is returned. "
                  "What stops a second mint displacing the first is now that statement, which "
                  "`set identity: displace an identity a set holds` proves.",
    },
    {
        # The identity has to reach the file or it reaches nobody.
        'name': "set identity: leave it out of the exported header",
        'file': "src/mcp.rs",
        'find': "        memory_file::write_header(scope, set_id, &mut body)",
        'replace': "        memory_file::write_header(scope, None, &mut body)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Adoption is what stops the hard cut being fragile: a store built from a file written before
        # identities existed gets ours on its next import rather than an invented rival.
        'name': "set identity: ignore the identity a file offers",
        'file': "src/mcp.rs",
        'find': "        let adopted = match validated.set_id {",
        'replace': "        let adopted: Option<String> = None;\n        let _unused = match validated.set_id {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # An absent identity must serialise to nothing at all. A field written as `null` would move
        # the checksum of every record already holding a cross-set link -- seven in the live set --
        # turning a compatible addition into a rewrite of everyone's records.
        'name': "link identity: write an absent identity as null",
        'file': "src/memory/link.rs",
        'find': '        #[serde(default, skip_serializing_if = "Option::is_none")]\n        set_id: Option<SetId>,',
        'replace': "        #[serde(default)]\n        set_id: Option<SetId>,",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # Authoring fills the identity in; a record arriving from a file must be written as its
        # author wrote it. Enriching on the way in is the receiver-side rewrite `absorb` removed.
        'name': "link identity: stop recording the target set's identity at authoring",
        'file': "src/mcp.rs",
        'find': "        self.identify_link_targets(&mut links)?;",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Inventing one for a set this machine does not hold would be asserting an identity nobody
        # authored -- and would make two sets sharing a name resolve to whichever is loaded.
        'name': "link identity: invent an identity for a set this machine does not hold",
        'file': "src/mcp.rs",
        'find': "            if let Some(store) = self.stores.get(&named) {",
        'replace': "            if let Some(store) = self.stores.get(&named).or_else(|| self.stores.values().next()) {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The store has to persist it, or it is minted and lost on the next read.
        'name': "link identity: drop it on the way into the store",
        'file': "src/store.rs",
        'find': "                set_id.map(|id| id.to_string()),",
        'replace': "                Option::<String>::None,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Since `set-settled-at-first-write` every set claims its identity on its first write, shared or
        # `local`. Confining it to `local` again restores the 2026-09-14 rule that was reversed.
        'name': "set identity: claim one on local's first write only",
        'file': 'src/store.rs',
        'find': '        self.mint_set_id()?;\n        if self.would_destroy_an_unrebuildable_vector(memory)? {',
        'replace': '        if self.scope == Scope::Local {\n            self.mint_set_id()?;\n        }\n        if self.would_destroy_an_unrebuildable_vector(memory)? {',
        'tests': ['store'],
        'expect': "caught",
    },
    {
        # Without the claim a set holding records can have no identity, which deciding whether a stored
        # link is same-set relies on -- and `local`, never created, could never acquire one at all.
        'name': 'set identity: never claim one on a write',
        'file': 'src/store.rs',
        'find': '        self.mint_set_id()?;\n        if self.would_destroy_an_unrebuildable_vector(memory)? {',
        'replace': '        if self.would_destroy_an_unrebuildable_vector(memory)? {',
        'tests': ['store'],
        'expect': "caught",
    },
    {
        # Redundant for *refusing* -- the occupied-destination guard catches it too -- and not
        # redundant for *diagnosing*, which is why the test asserts the wording. Deleting it turns
        # "you named the file being absorbed" into "a file is in the way".
        'name': "absorb: stop naming the file the safety copy collided with",
        'file': "src/mcp.rs",
        'find': "        // the caller, and nothing else stops the safety copy landing on the file being read.\n        if save_to == path {",
        'replace': "        // the caller, and nothing else stops the safety copy landing on the file being read.\n        if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # One guard, three callers, so it is pinned per call site rather than once. That is also the
        # shape that catches the failure a shared helper makes most likely: a single operation
        # losing the call while the others keep it.
        'name': "restore: skip the occupied-destination check",
        'file': "src/mcp.rs",
        'find': "                 ones.\",\n                path.display()\n            ));\n        }\n        if let Some(refusal) = occupied_safety_copy(save_to) {",
        'replace': "                 ones.\",\n                path.display()\n            ));\n        }\n        if let Some(refusal) = Option::<String>::None {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "absorb: skip the occupied-destination check",
        'file': "src/mcp.rs",
        'find': "                 is *this* set's current contents being put aside.\",\n                path.display()\n            ));\n        }\n        if let Some(refusal) = occupied_safety_copy(save_to) {",
        'replace': "                 is *this* set's current contents being put aside.\",\n                path.display()\n            ));\n        }\n        if let Some(refusal) = Option::<String>::None {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "drop: skip the occupied-destination check",
        'file': "src/mcp.rs",
        'find': "            if let Some(refusal) = occupied_safety_copy(path) {",
        'replace': "            if let Some(refusal) = Option::<String>::None {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The whole of `folding-is-lowercase-not-casefold`. Lowercase is a partial normalisation:
        # a caller cannot treat matching as byte identity, and it misses what a non-English author
        # actually writes.
        'name': "keyword: fold the lookup key by lowercasing",
        'file': "src/memory/text.rs",
        'find': "    let normalised: String = text.nfkc().collect();\n    caseless::default_case_fold_str(&normalised)\n        .nfkc()\n        .collect()",
        'replace': "    text.to_lowercase()",
        'tests': ["memory_model", "sharing_preferences"],
        'expect': "caught",
    },
    {
        # Order, not parts. Folded before it is normalised, U+1D2C stays `A` and never reaches `a`;
        # 664 characters below U+30000 behave this way, so this is a wide silent loss.
        'name': "keyword: fold before normalising rather than after",
        'file': "src/memory/text.rs",
        'find': "    let normalised: String = text.nfkc().collect();\n    caseless::default_case_fold_str(&normalised)\n        .nfkc()\n        .collect()",
        'replace': "    caseless::default_case_fold_str(text).nfkc().collect()",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # A store written under the old rule holds keys the query path can no longer match, so its
        # keyword search silently returns less than it should.
        'name': "keyword: skip refolding a store written under the old rule",
        'file': "src/store.rs",
        'find': "        if recorded.as_deref() == Some(RULE) {\n            return Ok(());\n        }",
        'replace': "        if recorded.is_none() || recorded.as_deref() == Some(RULE) {\n            return Ok(());\n        }",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # `a-superseded-by-naming-an-absent-id-hides-its-memory` again, from the other end: deciding
        # at each step instead of at the tip. It reads as the safer guard and is not -- an id this
        # store holds no record of can still be superseded in turn by one it does, and refusing to
        # walk strands a reader whose replacement is in the result list.
        'name': "resolution: refuse to walk into an id this store does not hold",
        'file': "src/store.rs",
        'find': "                    replacements.push(match &link.target {\n                        LinkTarget::SameSet(target) => Some(*target),",
        'replace': "                    replacements.push(match &link.target {\n                        LinkTarget::SameSet(target) if self.holds_record(*target)? => Some(*target),\n                        LinkTarget::SameSet(_) => None,",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # Walkable is not returnable. A chain that *ends* on a tombstone has no replacement to hand
        # back, and naming it as a tip drops the memory that yielded to it -- the same disappearance
        # as the absent id, reached by forgetting rather than by a hand edit.
        'name': "resolution: name an unfetchable tip instead of reporting it",
        'file': "src/store.rs",
        'find': "                if self.holds_memory(current)? {\n                    tips.push(current);\n                } else if self.holds_record(current)? {",
        'replace': "                if true {\n                    tips.push(current);\n                } else if self.holds_record(current)? {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # `a-cross-set-dispute-carries-score-to-the-memory-making-it`. Without the condition the
        # claimant is one hop from the memory it attacks and inherits accordingly -- the permanent
        # slot in every recall that `cross-set-references.md` declined to give it. Measured at 0.192
        # against a control of 0, so only a test with a control sees it.
        'name': "edges: walk a standing-changing edge that reaches into another set",
        'file': "src/store.rs",
        'find': "                if !(DISPUTING_LABELS.contains(&label) && target_set.is_some()) {\n                    let set = match target_set.as_deref() {",
        'replace': "                if true {\n                    let set = match target_set.as_deref() {",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # The other direction: dropping every cross-set edge from the walk, not only the two that
        # change standing. A descriptive link across sets is the ordinary case -- all three cross-set
        # edges in the live sets are descriptive -- and an unscoped recall is supposed to cross it.
        'name': "edges: refuse to walk any edge that names another set",
        'file': "src/store.rs",
        'find': "                if !(DISPUTING_LABELS.contains(&label) && target_set.is_some()) {",
        'replace': "                if target_set.is_none() {",
        'tests': ["cross_store", "recall"],
        'expect': "caught",
    },
    {
        # `superseded-beyond-this-set-is-reported-when-the-set-is-loaded`. With the upgrade skipped
        # every concession naming another set reports as out of reach, including one whose set is in
        # the same call -- which is the defect, and it is silent: the memory still comes back.
        'name': "resolution: ignore a concession whose set is being searched",
        'file': "src/store.rs",
        'find': "    for (set, set_id, target) in store.conceded_to_elsewhere(id)? {",
        'replace': "    for (set, set_id, target) in Vec::new() {",
        'tests': ["cross_store", "mcp"],
        'expect': "caught",
    },
    {
        # Two loaded sets may hold one id, and the keyword path is the one that still returns both
        # (`two-loaded-sets-sharing-one-id-collapse-to-one-memory` names it as the contrast). Keying
        # the substitution's dedup on the id alone collapses them here too -- which happened while
        # writing this, and the test below is what caught it.
        'name': "keyword hits: dedup a substitution on the id alone",
        'file': "src/store.rs",
        'find': '        if !taken.insert((scope_key(&hit.scope), hit.memory.id)) {',
        'replace': '        if !taken.insert((String::new(), hit.memory.id)) {',
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # `two-loaded-sets-sharing-one-id-collapse-to-one-memory`. The scan is where a memory enters
        # the ranking, and identifying it by id alone is the defect exactly: two sets holding one id
        # become one candidate, and the survivor is whichever store was scanned last.
        'name': "recall: identify a scanned memory by its id alone",
        'file': "src/store.rs",
        'find': "            scored.push((\n                hit.similarity,\n                Located {\n                    store: index,",
        'replace': "            scored.push((\n                hit.similarity,\n                Located {\n                    store: 0,",
        'tests': ["cross_store", "recall"],
        'expect': "caught",
    },
    {
        # The locating rule for a walkable edge. A named target lives in the store holding that set;
        # treating it as local attaches the edge to whatever the writing store happens to hold under
        # that id, which under a shared id is a memory the edge never pointed at.
        'name': "edges: attach a named edge to the store that wrote it",
        'file': "src/store.rs",
        'find': "                    Some(name) => holder_of(stores, name, set_id)?,",
        'replace': "                    Some(_) => Some(index),",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # `export-may-name-a-file`. The explicit form exists precisely so that nothing appends the
        # reserved directory, because nothing auto-discovers a backup or a synced folder. Appending
        # it anyway writes the set where it was not asked for and reports success with the path.
        # The write end since 2026-09-26, when `several-memory-files-per-directory` split finding a
        # file into one method per end; the read end is the entry after this one.
        'name': "destination: append the reserved directory to an explicit file too",
        'file': "src/mcp.rs",
        'find': "                    None => new_file_in(&home, scope),\n                }\n            }\n            Self::File(file) => Ok(file.clone()),",
        'replace': "                    None => new_file_in(&home, scope),\n                }\n            }\n            Self::File(file) => Ok(file.join(RESERVED_DIRECTORY).join(\"memories.json\")),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The read end of the entry above: an explicit import must read the file the caller named
        # and nothing appended to it.
        'name': "destination: read an explicit file from inside a reserved directory under it",
        'file': "src/mcp.rs",
        'find': "                locate_in(&home, scope, identity)?.ok_or_else(|| absent_from(&home, scope))\n            }\n            Self::File(file) => Ok(file.clone()),",
        'replace': "                locate_in(&home, scope, identity)?.ok_or_else(|| absent_from(&home, scope))\n            }\n            Self::File(file) => Ok(file.join(RESERVED_DIRECTORY).join(\"memories.json\")),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The constraint the design attaches to the explicit form, and it binds both ends for two
        # different reasons. Writing: anything under `scatterhoard_mem` is read as this project's set
        # from its path alone, so a file hand-placed there enters that namespace by a route the
        # export gate's unexpected-contents warning was not written to cover. Reading: the well-known
        # file is reachable by both forms, and the forms are two reconciliation targets -- so one
        # file named both ways would carry two records of what was last agreed.
        'name': "destination: let an explicit file reach into the reserved directory",
        'file': "src/mcp.rs",
        'find': "                if parent.file_name() == Some(std::ffi::OsStr::new(RESERVED_DIRECTORY)) {",
        'replace': "                if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Two destinations resolved by precedence rather than refused. A precedence rule is invisible
        # at the call site, so the losing argument is ignored in silence -- and for a destination that
        # means writing a set somewhere the caller did not mean it to go.
        'name': "export: accept two destinations and let one win",
        'file': "src/mcp.rs",
        'find': "        if brought.len() > 1 {",
        'replace': "        if brought.len() > 2 {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # An explicit file is its own replica. Keying its base on the parent merges two sets backed
        # up into one synced folder into a single target, so each import is handed the other file's
        # ancestor -- silently, since a base is never reported.
        'name': "destination: key an explicit file's agreement on its parent directory",
        'file': "src/mcp.rs",
        'find': "            Self::File(file) => file.to_string_lossy().into_owned(),",
        'replace': "            Self::File(file) => file.parent().unwrap_or(file).to_string_lossy().into_owned(),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The response's own statement of which form ran. `written` would still carry the path, so a
        # reader could infer it -- which is the point: an inference is not a statement, and this
        # mutation is what proves the field is asserted rather than merely present.
        'name': "export: report every write as the directory form",
        'file': "src/mcp.rs",
        'find': "            Self::File(_) => \"file\",",
        'replace': "            Self::File(_) => \"directory\",",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The flat schema publishes every action's arguments at the top level, so a client that fills
        # in the whole shape sends the ones its action does not use as `null`. Reading that as
        # supplied refuses an ordinary directory export for naming two destinations.
        'name': "arguments: count an explicit null as an argument that arrived",
        'file': "src/mcp.rs",
        'find': "    !matches!(arguments.get(name), None | Some(Value::Null))",
        'replace': "    arguments.get(name).is_some()",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The half of the round trip that has to agree with the other half. A target is where two
        # replicas last agreed, so an import keying on the file's parent looks for an ancestor the
        # export never wrote: identical content with no base reconciles as `converged` rather than
        # `unchanged`, which is a settled set reporting itself as freshly reconciled every time.
        'name': "import: reconcile an explicit file against its directory's agreement",
        'file': "src/mcp.rs",
        'find': "        let target = source.target();",
        'replace': "        let target = path.parent().unwrap_or(&path).to_string_lossy().into_owned();",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # And the response's statement of which form ran. It decides which recorded agreement the
        # import reconciled against, so a reader comparing two imports of one set cannot tell them
        # apart without it.
        # **Anchored on the `outcomes` line, which only `import` has.** `restore_set` carries an
        # identical `"path_form": source.form(),` and sits earlier in the file, so the bare line
        # retargeted this mutation onto restore -- where it survived, and reported as a defect in a
        # guard that was never touched. `--only` runs had passed because restore did not exist yet.
        'name': "import: report every read as the directory form",
        'file': "src/mcp.rs",
        'find': "            \"path_form\": source.form(),\n            // Present only when this import",
        'replace': "            \"path_form\": \"directory\",\n            // Present only when this import",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `set-drop`, and the rule the whole operation turns on: a destructive operation writes the
        # set somewhere the caller named *before* destroying it. Dropping without the copy is the
        # one outcome the design refuses outright -- there is no flag for it, so there must be no
        # path to it either.
        'name': "drop: destroy the set without writing the safety copy",
        'file': "src/mcp.rs",
        'find': "                    copy = Some(Self::write_memory_file(destination, path, &rendered.body)?);",
        'replace': "                    let _ = &rendered.body;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The corruption case, which the natural implementation gets exactly backwards. `?` here
        # reads as ordinary care and is the defect: the memories are already unreadable, so refusing
        # saves nothing and blocks the repair. This mutation is the shape a later editor would most
        # plausibly introduce while tidying.
        # Anchored past the arm itself: `restore_set` has the identical arm and comes first in the
        # file, so the bare line landed there instead.
        'name': "drop: refuse when the store cannot be read, instead of proceeding",
        'file': "src/mcp.rs",
        'find': "                Err(error) => unreadable = Some(error),",
        'replace': "                Err(error) => return Err(error),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # And its opposite, which must stay a refusal. The store is fine and the memories are
        # recoverable, so a destination that cannot be written is a real failure of the save --
        # proceeding here would drop a set that was perfectly saveable.
        'name': "drop: proceed when the safety copy cannot be written",
        'file': "src/mcp.rs",
        'find': "                    copy = Some(Self::write_memory_file(destination, path, &rendered.body)?);",
        'replace': "                    copy = Self::write_memory_file(destination, path, &rendered.body).ok();",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The unmissable half of the corruption case. Proceeding silently is worse than refusing:
        # the caller is told the set was dropped and has no way to learn that nothing was saved.
        # Anchored on drop's own wording, since `restore_set` opens the same block earlier.
        'name': "drop: proceed with no copy and say nothing about it",
        'file': "src/mcp.rs",
        'find': "        if let Some(error) = unreadable {\n            result[\"no_safety_copy\"] = json!(format!(\n                \"The set was dropped",
        'replace': "        if let Some(error) = unreadable.filter(|_| false) {\n            result[\"no_safety_copy\"] = json!(format!(\n                \"The set was dropped",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Unloading without unlinking leaves the set gone for this session and back on the next
        # start, which is the confusing middle state rather than either endpoint.
        'name': "drop: unload the set but leave its database on disk",
        'file': "src/mcp.rs",
        'find': "            std::fs::remove_file(database).map_err(|error| {",
        'replace': "            Ok::<(), std::io::Error>(()).map_err(|error: std::io::Error| {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `set-restore`, and the distinction the whole operation rests on. Keeping what the file does
        # not carry turns restore into import: it merges stale backup content into a live store and
        # produces a set that never existed at any moment in time.
        'name': "restore: keep what the file does not carry, making it an import",
        'file': "src/store.rs",
        'find': "        transaction.execute(\"DELETE FROM memories\", [])?;\n        transaction.execute(\"DELETE FROM links\", [])?;",
        'replace': "        transaction.execute(\"DELETE FROM links\", [])?;",
        'tests': ["sets", "store"],
        'expect': "caught",
    },
    {
        # The store half of `memory-file-read-silently-discards-tombstones`.
        #
        # **What this proves narrowed on 2026-08-26 and the mutation caught the moment it did.**
        # `write_memory` now clears any tombstone for an id it writes -- the fix for
        # `an-id-can-hold-both-a-memory-and-a-tombstone` -- so the overlapping case, a memory the
        # store forgot and the file carries live, is handled record by record and this statement no
        # longer proves it. What it still does, and nothing else does, is clear a tombstone for an id
        # the file never mentions: nothing writes that id, so nothing clears it. The guard did not
        # become redundant, it became **uncovered**, and it now has its own test.
        'name': "restore: leave the store's own tombstones beside the file's memories",
        'file': "src/store.rs",
        'find': "        transaction.execute(\"DELETE FROM tombstones\", [])?;",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The file half of the same defect. A restored set that dropped the file's tombstones has
        # forgotten what it forgot, and the next export resurrects every one of them elsewhere.
        'name': "restore: discard the file's tombstones instead of writing them",
        'file': "src/store.rs",
        'find': "                    replaced.forgotten += 1;",
        'replace': "                    replaced.forgotten += 0;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `never keep a reconciliation base you cannot prove`. A base for a record the restored set
        # no longer holds resolves as `Anomaly` -- a state no supported operation produces -- so
        # keeping them makes restore manufacture the report that means something went wrong outside
        # the tools.
        'name': "restore: keep every target's agreement across a replacement",
        'file': "src/store.rs",
        'find': "        transaction.execute(\"DELETE FROM bases\", [])?;",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The correctness requirement the design states and nothing else enforces: both paths come
        # from the caller, so only this stops the safety copy landing on the file being restored
        # from -- destroying the good copy in the act of protecting the bad one.
        'name': "restore: save over the very file being restored from",
        'file': "src/mcp.rs",
        'find': "        // and are not a new defect of this one.\n        if save_to == path {",
        'replace': "        // and are not a new defect of this one.\n        if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The agreement the restore just established. Without it the next import from the same file
        # reconciles with no ancestor and re-decides every record it already matches.
        'name': "restore: replace the contents without recording the agreement",
        'file': "src/mcp.rs",
        'find': "        store.set_bases(&target, &bases).map_err(|error| {",
        'replace': "        store.set_bases(&target, &[]).map_err(|error| {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Restore's own copy of the corruption rule, and the case it matters most for: a store too
        # damaged to read is what restore *exists* for, so refusing there withholds the repair from
        # the only situation that needs it. Nothing proved this until the four retargeted mutations
        # showed restore's copies of these branches were untested.
        'name': "restore: refuse when the old contents cannot be read, instead of proceeding",
        'file': "src/mcp.rs",
        'find': "            Err(error) => unreadable = Some(error),\n        }\n\n        let target = source.target();",
        'replace': "            Err(error) => return Err(error),\n        }\n\n        let target = source.target();",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # And the unmissable half. A restore that silently made no copy tells the caller their set
        # was replaced and leaves them no way to learn that what it replaced is gone.
        'name': "restore: replace with no copy and say nothing about it",
        'file': "src/mcp.rs",
        'find': "        if let Some(error) = unreadable {\n            result[\"no_safety_copy\"] = json!(format!(\n                \"The set was restored",
        'replace': "        if let Some(error) = unreadable.filter(|_| false) {\n            result[\"no_safety_copy\"] = json!(format!(\n                \"The set was restored",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `a-scope-in-a-response-is-not-the-spelling-the-schema-publishes`. The surface publishes
        # `local` / `shared:<name>`; responses emitted `Scope`'s derived object form, so a caller
        # reading the schema learned one spelling and met another in every result. Both parse, which
        # is why this was `Misleads` rather than worse -- and is also why only an assertion on the
        # spelling can catch a regression.
        'name': "wire: publish a scope in a response as the derived object form",
        'file': "src/mcp.rs",
        'find': "        serializer.serialize_str(&self.0.as_key())",
        'replace': "        self.0.serialize(serializer)",
        'tests': ["mcp", "sets", "approval"],
        'expect': "caught",
    },
    {
        # And the half that must not move with it. Every `memories.json` on disk carries the object
        # form in its header; changing that is a format change with a version bump, not a surface
        # fix. A later tidying pass that "makes them consistent" is what this notices -- so the
        # mutation writes the scalar into the header, which is exactly what such a pass would do.
        'name': "wire: publish the file header's scope as the scalar too",
        'file': "src/memory_file.rs",
        'find': "        serde_json::to_string(&header).expect(SERIALISES)",
        'replace': "        serde_json::to_string(&serde_json::json!({\"format_version\": header.format_version, \"scope\": header.scope.as_key()})).expect(SERIALISES)",
        'tests': ["sets", "memory_file"],
        'expect': "caught",
    },
    {
        # `memory-file-read-silently-discards-tombstones`, the reading half. This is the exact edit
        # that caused it: when the format grew a second record shape, `read` was given the minimum
        # change that kept a `Vec<Memory>` compiling, and every tombstone went on the floor with no
        # count, flag or error to say so.
        # Anchored on the sort rather than on the match arm: dropping the arm leaves `tombstones`
        # with no inferred type, so the mutation would be "caught" by a compile error, which proves
        # nothing about any test. Clearing the vector reproduces the old behaviour exactly -- a read
        # that hands back no tombstones -- and has to be caught by an assertion.
        'name': "memory file: read a file's memories and drop its tombstones",
        'file': "src/memory_file.rs",
        'find': "    tombstones.sort_by_key(|tombstone| tombstone.id);",
        'replace': "    tombstones.clear();",
        'tests': ["memory_file", "sets"],
        'expect': "caught",
    },
    {
        # And the writing half, which had the same gap for a different reason: `write` only ever
        # iterated `memories`, because until now there was nowhere for a tombstone to come from. A
        # set written this way has forgotten what it forgot.
        'name': "memory file: write a set's memories and drop its tombstones",
        'file': "src/memory_file.rs",
        'find': "        .chain(\n            set.tombstones\n                .iter()\n                .map(|tombstone| Record::Tombstone(tombstone.clone())),\n        )",
        'replace': "",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # A file is ordered by id, which is what makes its diffs reviewable and puts a duplicate id
        # next to its twin. Sorting by shape instead orders nothing a reader cares about, and makes
        # every diff that forgets a memory move a line to the end of the file.
        #
        # **Replaces `file: stop sorting memories on write`**, whose anchor moved when `write` was
        # rewritten to merge both record shapes. Same guard, new location, and now covering the
        # ordering of tombstones as well -- so this is one mutation rather than two on one line.
        'name': "memory file: order a written file by record shape rather than by id",
        'file': "src/memory_file.rs",
        'find': "    records.sort_by_key(|record| file_position(record.id()));",
        'replace': "",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # Id order put every new memory at the end of the file, where two branches that both added
        # memories always collided (`concurrent-additions-collide-at-the-end-of-the-memory-file`).
        # A position that is the id again is exactly that order, and would pass any test that only
        # asked for *a* stable order.
        'name': "memory file: position records by id rather than by a hash of it",
        'file': "src/memory_file.rs",
        'find': "    (crate::checksum::of_bytes(id.to_string().as_bytes()), id)",
        'replace': "    (0, id)",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # Export writes what the store streams, so the file's order is decided here and not by the
        # writer, which cannot sort what it has not yet seen.
        'name': "store: stream records for export in id order",
        'file': "src/store.rs",
        'find': "             ORDER BY 2, 1\",",
        'replace': "             ORDER BY 1\",",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # A merged file in id order would be reordered wholesale by the next export.
        'name': "merge: write the merged file in id order",
        'file': "src/merge.rs",
        'find': "    ids.sort_unstable_by_key(|id| memory_file::file_position(*id));",
        'replace': "    ids.sort_unstable();",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # The store half. A whole-set import that takes only `memories` leaves the store having
        # forgotten what the set forgot, and the next export resurrects every one of them elsewhere.
        'name': "store: import a whole set's memories and drop its tombstones",
        'file': "src/store.rs",
        'find': "            .chain(\n                set.tombstones\n                    .iter()\n                    .map(|tombstone| crate::memory_file::Record::Tombstone(tombstone.clone())),\n            )",
        'replace': "",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # `target-identity-is-an-unnormalised-path-string`. A target was the caller's path as typed,
        # so one directory could be four targets with four independent base sets -- and an import
        # under an unfamiliar spelling reconciled the whole file with no ancestor, in silence.
        'name': "target: key a reconciliation target on the path as the caller typed it",
        'file': "src/store.rs",
        'find': "        let path = Path::new(target);\n        if let Ok(resolved) = path.canonicalize() {",
        'replace': "        if true {\n            return target.to_owned();\n        }\n        let path = Path::new(target);\n        if let Ok(resolved) = path.canonicalize() {",
        'tests': ["sets", "store"],
        'expect': "caught",
    },
    {
        # The parent fallback: an explicit `file` target names a file `export` is about to *create*,
        # so at the moment the caller computes its target string the path does not resolve yet.
        #
        # **Subsumed on every shipped path, traced rather than assumed.** `export_set` computes the
        # target before writing, but the value that reaches the table goes through `set_bases`, which
        # normalises again -- and by then the file exists, so the canonical form is stored either
        # way. The pre-write string is used only to compare against `known_targets`, and for a file
        # being created for the first time the honest answer is *unrecognised* under either spelling.
        #
        # The line stays, because it is what makes normalisation total and independent of call
        # order. What currently saves the export path is that one caller happens to normalise after
        # the write rather than before it, which is not a property anybody would think to preserve.
        'name': "target: give up on a path whose own name does not resolve yet",
        'file': "src/store.rs",
        'find': "        if let (Some(parent), Some(name)) = (path.parent(), path.file_name())\n            && let Ok(resolved) = parent.canonicalize()",
        'replace': "        if false\n            && let (Some(parent), Some(name)) = (path.parent(), path.file_name())\n            && let Ok(resolved) = parent.canonicalize()",
        'tests': ["sets", "store"],
        'expect': "survives",
        'reason': "`set_bases` normalises again at the store boundary, after the file it names has "
                  "been written, so the canonical form is stored whether or not this branch runs. "
                  "It holds if a caller ever normalises once and passes the result on, which is the "
                  "obvious way to write it.",
    },
    {
        # **The trap the design names by itself**: two raw keys that resolve to one key carry two
        # independent base sets, and wherever they disagree about a memory neither is the other's
        # ancestor. Taking either is a silent overwrite -- the one outcome reconciliation exists to
        # prevent -- so both are dropped. This is the tidy-looking edit that would undo it.
        'name': "migration: merge two spellings of one target instead of dropping both",
        'file': "src/store.rs",
        'find': "            if raws.len() > 1 {",
        'replace': "            if raws.len() > 2 {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # A target whose directory has gone must keep its spelling and its agreement: an unmounted
        # volume comes back to the same path. Dropping the unresolvable ones is the other tidy-looking
        # edit, and it discards ancestry that is still good.
        'name': "migration: drop a target that no longer resolves",
        'file': "src/store.rs",
        'find': "            let raw = &raws[0];\n            if *raw != normalised {",
        'replace': "            let raw = &raws[0];\n            if *raw == normalised {\n                transaction.execute(\"DELETE FROM bases WHERE target = ?1\", params![raw])?;\n            }\n            if *raw != normalised {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The load-bearing half, which normalisation does not retire: canonicalising collapses
        # spellings but does not survive a checkout being renamed or moved, which orphans every base
        # and arrives looking like a location never seen before. Import reconciled against no
        # ancestor in silence until 2026-08-26.
        'name': "import: reconcile against no agreement without saying so",
        'file': "src/mcp.rs",
        'find': "        if !known.is_empty() && !known.contains(&target) {\n            result[\"note\"] = json!(format!(\n                \"this set has agreed with {} before;",
        'replace': "        if false {\n            result[\"note\"] = json!(format!(\n                \"this set has agreed with {} before;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `an-id-can-hold-both-a-memory-and-a-tombstone`. `forget` deletes the memory row as it
        # writes the tombstone; without this statement the pair is asymmetric, and a memory arriving
        # for a tombstoned id leaves both rows. `stream_records` then emits that id twice and the
        # file format refuses a duplicate -- this project writing a file its own reader will not
        # read, from a sequence as ordinary as `git checkout` of an earlier commit and an import.
        'name': "store: leave a tombstone in place when its memory is written back",
        'file': "src/store.rs",
        'find': "    connection.execute(\"DELETE FROM tombstones WHERE id = ?1\", params![id])?;",
        'replace': "",
        'tests': ["sets", "store"],
        'expect': "caught",
    },
    {
        # And the half that keeps the invariant's repair from hiding the defect it was found with.
        # A `TakeFile` whose incoming version predates the one it replaces is never an ordinary
        # pull; without the report, fixing the tombstone rule alone turns a loud corruption -- an
        # export that half-fails and writes an unreadable file -- into a silent resurrection.
        'name': "import: take a record backwards without saying so",
        'file': "src/mcp.rs",
        'find': "                if outcome == reconcile::Outcome::TakeFile\n                    && let Some((_, held_at)) = held\n                    && record.as_of() < held_at",
        'replace': "                if outcome == reconcile::Outcome::TakeFile\n                    && let Some((_, held_at)) = held\n                    && record.as_of() < held_at\n                    && false",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The direction of the comparison, which is the whole of what it says. Reversed, it reports
        # every ordinary pull and nothing else -- noise where the signal was, and the report stops
        # being worth reading at exactly the moment it matters.
        'name': "import: report an ordinary pull as a record taken backwards",
        'file': "src/mcp.rs",
        'find': "                    && record.as_of() < held_at",
        'replace': "                    && record.as_of() > held_at",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `as_of` over both record shapes is what makes a resurrection compare like any other edit:
        # the memory coming back is older than the tombstone that replaced it. Reading a tombstone
        # as the epoch instead makes every resurrection look like an ordinary pull.
        'name': "record: read a tombstone as having no moment of its own",
        'file': "src/memory_file.rs",
        'find': "            Self::Tombstone(tombstone) => tombstone.forgotten.at,",
        'replace': "            Self::Tombstone(_) => jiff::Timestamp::UNIX_EPOCH,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # One tip is what makes a chain a chain. Accepting the first of several turns the
        # duplicate-chain hazard -- one identity split in two, which the design says nothing
        # downstream can detect -- into a confident answer picked arbitrarily.
        'name': "author_name: take the first tip when a chain has several",
        'file': "src/store.rs",
        'find': "            Resolution::Superseded { tips, .. } if tips.len() == 1 => tips[0],",
        'replace': "            Resolution::Superseded { tips, .. } if !tips.is_empty() => tips[0],",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The reserved keyword is the only thing separating an identity record from a memory that
        # happens to contain a name. Without the check, any memory's content reads as an author.
        'name': "author_name: treat any memory as an identity record",
        'file': "src/store.rs",
        'find': "        let Some(memory) = self.get(tip)? else {\n            return Ok(None);\n        };\n        if !memory.is_identity_record() {",
        'replace': "        let Some(memory) = self.get(tip)? else {\n            return Ok(None);\n        };\n        if false {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # Identity records are not knowledge, and every set holds one, so letting them be embedded
        # puts a record matching nothing into the ranking of every semantic recall.
        'name': "embed: give identity records vectors like any other memory",
        'file': "src/store.rs",
        'find': "            .copied()\n            .filter(|memory| !memory.is_identity_record())",
        'replace': "            .copied()\n            .filter(|_| true)",
        'tests': ["mcp", "sets"],
        'expect': "caught",
    },
    {
        # `memories` answers *how much did this set take on*, and nobody wrote their own name as a
        # memory. Counting the identity record reports infrastructure as content.
        'name': "count: report identity records as memories",
        'file': "src/store.rs",
        'find': "            \"SELECT COUNT(*) FROM memories WHERE created_by <> id\",",
        'replace': "            \"SELECT COUNT(*) FROM memories\",",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Reporting what a rebuild *intended* rather than what it produced is the shape that hid
        # `project-stores-get-no-embedder`. Input and output now differ only by memories the model
        # cannot represent -- identity records left the input on 2026-09-22, in `gaps` -- and only
        # `unrepresentable` builds one. Run against `sets` alone this survived while recorded as
        # caught: `the-reembed-count-mutation-no-longer-changes-anything`.
        'name': "reembed: report the memories offered rather than the vectors written",
        'file': "src/store.rs",
        'find': "        let embedded = self.embed_all(&borrowed)?;",
        'replace': "        self.embed_all(&borrowed)?;\n        let embedded = Outcomes { vectors: borrowed.len(), unrepresentable: 0 };",
        'tests': ["sets", "unrepresentable"],
        'expect': "caught",
    },
    {
        # The first identity record in a set has nothing earlier to point at, so it points at
        # itself. Pointing anywhere else is a dangling reference in the one field that cannot have
        # one.
        'name': "self_attributed: attribute the first identity record to something else",
        'file': "src/memory/memory.rs",
        'find': "        let mut memory = Self::new(content, confidence, origin, Attribution { by: id, at });\n        memory.id = id;",
        'replace': "        let mut memory = Self::new(content, confidence, origin, Attribution { by: id, at });\n        memory.id = MemoryId::generate();",
        'tests': ["approval"],
        'expect': "caught",
    },
    {
        # A row naming a record this copy of the set does not hold must not be adopted: every write
        # would then point at nothing, which is the one failure holding `by` as a reference adds.
        'name': "author map: trust a row without checking the set holds the record",
        'file': "src/mcp.rs",
        # The find text names the line after the branch as well, because `            if held {` is
        # also a substring of a more deeply indented `if held {` elsewhere in this file -- and a
        # substring match would silently mutate that one instead, which no test here covers.
        'find': "            if held {\n                store.write_author_record(row.record).map_err(|error| {",
        'replace': "            if held || true {\n                store.write_author_record(row.record).map_err(|error| {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `create` names nobody since `set-settled-at-first-write` step 3: a shared set asks who you are at
        # its first write, because the name is how you are known where the set is published. Replaced the
        # mutation that minted at `create` where it used to record a name, which recording no longer exists
        # to be.
        'name': "create: name the set's author from the account name again",
        'file': 'src/mcp.rs',
        'find': '        // **Nor is an author named here',
        'replace': '        if let Some(name) = self.account_name.clone() {\n            let _ = self.stores.get_mut(&scope).expect("just added").author_record_or_mint(&name, jiff::Timestamp::now());\n        }\n        // **Nor is an author named here',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # The map is machine state living in one store, so there is never a question which copy
        # answers. `require_local` is what enforces that; the schema condition behind it is defence
        # in depth and has no observable effect on its own, which is why this pins the refusal.
        'name': "author map: answer it from any store rather than only the local one",
        'file': "src/store.rs",
        'find': "        if matches!(self.scope, Scope::Local) {\n            return Ok(());\n        }",
        'replace': "        return Ok(());\n        #[allow(unreachable_code)]\n        if false {}",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The floor is the whole of the format cut. Dropped back below the attribution change, a
        # pre-cut file is accepted and then fails on the first record with a malformed-id error --
        # a damage report where a refusal belongs, sending the reader to look for damage that is
        # not there.
        'name': "format gate: put the floor back below the attribution cut",
        'file': "src/memory_file.rs",
        'find': "pub const OLDEST_READABLE: &str = \"0.4.0\";",
        'replace': "pub const OLDEST_READABLE: &str = \"0.1.0\";",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        # `mint` is the operation that can fork a person, so it is the one that must refuse rather
        # than infer. Two identity records for one contributor are indistinguishable from two
        # contributors, and nothing downstream can report the mistake.
        'name': "mint: mint a second identity for someone this set already knows",
        'file': "src/store.rs",
        'find': "        if let Some(held) = self.author_record()? {\n            return Err(StoreError::AlreadyIdentified {",
        'replace': "        if let Some(held) = self.author_record()? && false {\n            return Err(StoreError::AlreadyIdentified {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `identify` takes an id precisely so it cannot be pointed at something that is not an
        # identity. Without the keyword check, an ordinary memory becomes the thing every later
        # write is attributed to.
        'name': "identify: accept an ordinary memory as an identity record",
        'file': "src/store.rs",
        'find': "            return Err(refuse(\"it is an ordinary memory, not an identity record\"));",
        'replace': "            let _ = refuse;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Superseding without forgetting is the half-answer everyone reaches for first: the old name
        # stays readable in every copy of the set, so a rename made to stop publishing under a name
        # achieves nothing.
        'name': "rename: supersede the old identity without forgetting it",
        'file': "src/store.rs",
        'find': "        self.forget(previous, replacement, at)?;",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The cache is repointed before the old record is forgotten, so no moment exists where this
        # store's answer to *which record is you* names a tombstone.
        #
        # **Recorded as surviving, with the reason.** Both writes happen inside one call and nothing
        # observes the store between them, so the ordering is unobservable from outside and no test
        # can distinguish it -- writing one that reached in mid-operation would be testing the
        # implementation rather than the behaviour. It is kept rather than deleted because the
        # ordering is load-bearing the moment anything does read between them: a concurrent reader,
        # or a later refactor that resolves an author while renaming. The corpus is where that
        # intent survives a refactor; a comment on the line is not consulted by anything.
        'name': "rename: forget the old identity before repointing the cache",
        'file': "src/store.rs",
        'find': "        self.write_author_record(replacement)?;\n        // Attributed to the identity doing the forgetting",
        'replace': "        self.forget(previous, replacement, at)?;\n        self.write_author_record(replacement)?;\n        // Attributed to the identity doing the forgetting",
        'tests': ["sets"],
        'expect': "survives",
    },
    {
        # *What was forgotten* and *what was never here* are different answers, and only one has a
        # next step. Lookup gave both the same empty list, which became load-bearing when `rename`
        # started forgetting the record every earlier memory's `by` names.
        'name': "lookup: report a forgotten id the same way as one that never existed",
        'file': "src/mcp.rs",
        'find': "        if let (Some(_), true) = (&params.id, hits.is_empty()) {",
        'replace': "        if false {",
        'tests': ["correct", "sets"],
        'expect': "caught",
    },
    {
        # `superseded_by` names the end of a chain and `incoming` names the next link. Dropping the
        # second leaves a forgotten record jumpable but not walkable: whatever replaced it directly
        # becomes unreachable going forward, which is the middle of every chain a `rename` makes.
        'name': "lookup: report a tombstone's chain end without its next link",
        'file': "src/mcp.rs",
        'find': "                        \"incoming\": incoming,",
        'replace': "",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The whole point of recording a target set's identity on a link. Matching the name first
        # defeats both purposes the field exists for: a stale name reaches nothing while the
        # identity sits unused in the same row, and a shared name reaches the *wrong* set.
        'name': "cross-set links: find the target's set by name even when the link names its identity",
        'file': "src/store.rs",
        'find': "    if let Some(wanted) = set_id {",
        'replace': "    if let Some(wanted) = set_id && false {",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # A stated identity is not overridden by a name. Falling back to the name when the identity
        # names no loaded set resolves against a set that merely shares the name -- which is worse
        # than not resolving, because it attaches one set's concession to another set's memory.
        'name': "cross-set links: fall back to the name when the stated identity is not loaded",
        'file': "src/store.rs",
        'find': "        return Ok(None);\n    }\n    Ok(stores\n        .iter()\n        .position(|store| store.scope.exports_to() == Some(set)))",
        'replace': "    }\n    Ok(stores\n        .iter()\n        .position(|store| store.scope.exports_to() == Some(set)))",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # The edge lives in the source's store, so only the layer holding every store can find it.
        # Without the fill, a cross-set link is visible from the side that wrote it and nowhere else.
        'name': "lookup: leave a cross-set reference invisible from the memory it points at",
        'file': "src/mcp.rs",
        'find': "                    format!(\"could not check what else points at this memory: {error}\")\n                })?;",
        'replace': "                    format!(\"could not check what else points at this memory: {error}\")\n                })?;\n            hit.referenced_from = Vec::new();",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Same rule as `holder_of`, in the other direction: a stated identity decides, so a row
        # naming this set by a stale name still points here and one naming another set does not.
        'name': "references: match a cross-set reference by name rather than by identity",
        'file': "src/store.rs",
        'find': "                (Some(stated), Some(ours)) => stated == ours,",
        'replace': "                (Some(_), Some(_)) => named.as_deref() == Some(target_set.as_str()),",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # Personal sets are included deliberately, and are the case this exists for: a shared memory
        # may not link into a personal set, so the only cross-boundary edge runs the other way.
        'name': "references: skip personal sets the way claims do",
        'file': "src/store.rs",
        'find': "    for store in stores {\n        if &store.scope == scope {\n            continue;\n        }\n        for edge in store.edges_into_another_set(id, target_set, target_set_id)? {",
        'replace': "    for store in stores {\n        if &store.scope == scope || store.scope.exports_to().is_none() {\n            continue;\n        }\n        for edge in store.edges_into_another_set(id, target_set, target_set_id)? {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Tombstones count as records. A set that has forgotten something has been written to by this
        # machine, so making it read-only would let `drop` skip the safety copy that holds them. It
        # guarded identity adoption too until `set-settled-at-first-write` made a held identity final.
        'name': "load modes: let a set holding only tombstones be made read-only",
        'file': "src/store.rs",
        'find': "            \"SELECT EXISTS(SELECT 1 FROM memories UNION ALL SELECT 1 FROM tombstones)\",",
        'replace': "            \"SELECT EXISTS(SELECT 1 FROM memories)\",",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The one state `status` exists to warn about here: the next write is refused rather than
        # guessing, and meeting that mid-write is the worst moment to be asked about set membership.
        'name': "author note: say nothing when the next write will have to ask",
        'file': "src/mcp.rs",
        'find': "    if must_ask {",
        'replace': "    if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A set that has been claimed says nothing, whatever name it attributes to: identity follows a set's
        # origin, and a note firing on a healthy set is one readers learn to skip. Re-anchored since step 3
        # removed the configured name the old mutation compared against.
        'name': 'author note: speak up for a set that has been claimed',
        'file': 'src/mcp.rs',
        'find': '    if claimed {\n        return None;\n    }',
        'replace': '    if claimed && false {\n        return None;\n    }',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # The durable map answers before the question is put, so a set whose cache is gone but whose
        # identity this machine has already chosen must not be reported as needing an answer.
        'name': "author note: ignore the durable map when deciding whether to ask",
        'file': "src/mcp.rs",
        'find': "            && store\n                .holds_record(row.record)",
        'replace': "            && false\n            && store\n                .holds_record(row.record)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The cap is what the whole index rests on: at 120 characters a hundred lines cost 0.9x the
        # convention catalog, and uncapped they cost whatever the memories happen to weigh. A cap
        # that silently stops binding is the failure this projection exists to prevent, wearing the
        # projection's own name.
        'name': "summary: let the line run to whatever the memory weighs",
        'file': "src/memory/text.rs",
        'find': "    if line.len() <= cap {",
        'replace': "    if true {",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # Last sentence end, not first. Taking the first spends a quarter of the budget on a memory
        # opening with a short sentence, and truncates to four characters on an `e.g.` -- and both
        # are silent, since a short line looks like a short memory.
        'name': "summary: cut at the first sentence end rather than the last that fits",
        'file': "src/memory/text.rs",
        'find': "    let cut = if let Some(end) = (0..room).rev().find(|at| ends_a_sentence(*at)) {",
        'replace': "    let cut = if let Some(end) = (0..room).find(|at| ends_a_sentence(*at)) {",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # A terminator ends a sentence only when whitespace follows. Without that, `0.5` and
        # `store.rs` are sentence ends -- and this corpus is full of both.
        'name': "summary: treat every dot as a sentence end",
        'file': "src/memory/text.rs",
        'find': "            && line.get(at + 1).is_some_and(|next| *next == ' ')\n",
        'replace': "",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # With no sentence to end on the line ends at a word. Cutting at the cap regardless is what
        # the check script did, and it is the one difference between the measured corpus and this.
        'name': "summary: cut mid-word at the cap instead of at the last word that fits",
        'file': "src/memory/text.rs",
        'find': "        line[..room].iter().rposition(|c| *c == ' ').unwrap_or(room)",
        'replace': "        room",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # A memory's interior is preserved exactly, newlines included; a *line* of an index cannot
        # be. Splitting on spaces alone keeps the newlines, and an index whose entries are each two
        # lines is not an index.
        'name': "summary: keep the memory's newlines in the line",
        'file': "src/memory/text.rs",
        'find': "        for (index, word) in text.split_whitespace().enumerate() {",
        'replace': "        for (index, word) in text.split(' ').enumerate() {",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # The projection is what the caller asked for, or it is not a choice. Serving the whole
        # record to a caller that asked for the index is the failure that costs nothing visible --
        # the answer is right, and only the budget is wrong.
        'name': "summary detail: return the whole record anyway",
        'file': "src/mcp.rs",
        'find': "            Detail::Full => self.to_json(),",
        'replace': "            Detail::Full | Detail::Summary => self.to_json(),",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # `recall-mechanisms.md`: a reduced level may be offered and may not become the default. A
        # caller that has not chosen cannot know what it gave up.
        'name': "summary detail: make the reduced level the default",
        'file': "src/mcp.rs",
        'find': "    #[default]\n    Full,\n    /// The id and one derived line, for building an index over many memories at once.\n    Summary,",
        'replace': "    Full,\n    /// The id and one derived line, for building an index over many memories at once.\n    #[default]\n    Summary,",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The echo is the whole reason the cap can be stated as a rule an author follows: it makes
        # the derived line visible at the moment of writing, with nobody counting characters.
        'name': "store response: drop the echoed summary line",
        'file': "src/mcp.rs",
        'find': "            \"summary\": memory.content.summary_as_data(),",
        'replace': "",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The marker comes out of the cap rather than being added past it. Appended, the cap stops
        # binding on exactly the lines that are already at it -- the ones a cap exists for.
        'name': "summary: append the cut marker past the cap instead of inside it",
        'file': "src/memory/text.rs",
        'find': "    let room = cap - 1;",
        'replace': "    let room = cap;",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # A cut line says it was cut, or an unmarked line means either `complete` or `cut at a
        # sentence` -- which is the ambiguity the marker exists to remove, moved rather than closed.
        'name': "summary: leave a cut line unmarked",
        'file': "src/memory/text.rs",
        'find': "    summary.push(CUT_MARKER);",
        'replace': "",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # The echo on the write that can still change the line. `hoard_memorize` shows the first
        # draft and amend is every try after it, which is the loop the field exists for -- and the
        # loop runs before export, which is the window amend is open in.
        'name': "amend: drop the echoed summary line",
        'file': "src/mcp.rs",
        'find': "        if changed.contains(&\"content\") {\n            result[\"summary\"] = json!(memory.content.summary_as_data());\n        }",
        'replace': "",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # Scoped to the write that moved the line, on the same rule as the in-link remark beside it:
        # a field that appears when nothing about it changed is one callers learn to skip.
        'name': "amend: echo the line on every amendment",
        'file': "src/mcp.rs",
        'find': "        if changed.contains(&\"content\") {",
        'replace': "        if true {",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # The served rule quotes a number the code enforces, and served prose cannot interpolate it.
        # A hand-written copy is only safe while something compares it to its source.
        'name': "conventions: let the served cap drift from the enforced one",
        'file': "src/memory/text.rs",
        'find': "pub const SUMMARY_CAP: usize = 120;",
        'replace': "pub const SUMMARY_CAP: usize = 100;",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        # `about-hoard-author-as-a-subject-keyword-silently-disables-embedding`. An identity record
        # is recognised by attributing itself; asking the keyword instead lets any caller who writes
        # it -- including a memory merely discussing identity records -- fall out of embedding.
        'name': "identity: recognise an identity record by its keyword again",
        'file': "src/memory/memory.rs",
        'find': "        self.created.by == self.id",
        'replace': "        self.keywords.iter().any(|k| k.as_str() == \"hoard-author\")",
        'tests': ["mcp", "store"],
        'expect': "caught",
    },
    # `one-unembeddable-memory-stops-the-store`, fixed 2026-09-11. A memory whose text holds no token
    # the model knows used to fail the batch it arrived in, and the sweep sends every gap through one
    # batch -- so one such memory stopped every later write and every recall by meaning.
    {
        # All zeros is the one outcome a text alone can cause, so it is a verdict about the text.
        'name': "unrepresentable: treat a vector of zeros as a fault again",
        'file': "src/embedder.rs",
        'find': "        Err(EmbeddingError::NoDirection) => Ok(Embedded::Unrepresentable),",
        'replace': "        Err(EmbeddingError::NoDirection) => Err(EmbedderError::from(EmbeddingError::NoDirection)),",
        'tests': ["embedder", "unrepresentable"],
        'expect': "caught",
    },
    {
        # The other direction: a broken model must not pass as a language gap.
        'name': "unrepresentable: call every bad vector unrepresentable",
        'file': "src/embedder.rs",
        'find': "        Err(other) => Err(EmbedderError::from(other)),",
        'replace': "        Err(_) => Ok(Embedded::Unrepresentable),",
        'tests': ["embedder", "unrepresentable"],
        'expect': "caught",
    },
    {
        # The strict form fails the whole batch on one unrepresentable text, which is the defect.
        'name': "unrepresentable: let the store use the strict embed",
        'file': "src/store.rs",
        'find': "        let outcomes = embedder.embed_each(&inputs)?;",
        'replace': "        let outcomes: Vec<Embedded> = embedder.embed(&inputs)?.into_iter().map(Embedded::Vector).collect();",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # Without a row, the sweep cannot tell a verdict from a gap and asks again on every call.
        'name': "unrepresentable: record no row for a verdict",
        'file': "src/store.rs",
        'find': "            Embedded::Unrepresentable => UNREPRESENTABLE.to_vec(),",
        'replace': "            Embedded::Unrepresentable => continue,",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # A verdict row is a row, so findability has to ask for a real vector.
        'name': "unrepresentable: count a verdict as findable by meaning",
        'file': "src/store.rs",
        'find': "\"SELECT 1 FROM vectors WHERE memory_id = ?1 AND length(vector) > 0\",",
        'replace': "\"SELECT 1 FROM vectors WHERE memory_id = ?1\",",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # An empty blob decodes to no components, which `Vector` refuses: reopening a store holding
        # a verdict would fail to load it.
        'name': "unrepresentable: load a verdict as though it were a vector",
        'file': "src/store.rs",
        'find': "            if blob.is_empty() {",
        'replace': "            if blob.is_empty() && false {",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # A rewrite into text the model cannot represent must not go on being found by its old vector.
        'name': "unrepresentable: keep the old vector cached after a rewrite",
        'file': "src/store.rs",
        'find': "                    self.vectors.remove(id);",
        'replace': "                    let _ = id;",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # A verdict binds the store to its model; counting only vectors releases it under *no
        # vectors, no model*, and a later model would skip every verdict-holding memory.
        'name': "unrepresentable: count only vectors when deciding whether a model binds the store",
        'file': "src/store.rs",
        'find': "            .query_row(\"SELECT count(*) FROM vectors\", [], |row| row.get(0))?;",
        'replace': "            .query_row(\"SELECT count(*) FROM vectors WHERE length(vector) > 0\", [], |row| row.get(0))?;",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # The write used to commit and then embed, so a failed embedding left the memory behind an
        # error that said it was not stored.
        'name': "unrepresentable: let a write proceed past a failed embedding",
        'file': "src/store.rs",
        'find': "        let embeddings = self.embed_for(&[memory])?;",
        'replace': "        let embeddings = self.embed_for(&[memory]).ok().flatten();",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        'name': "unrepresentable: stop saying the query was unrepresentable",
        'file': "src/store.rs",
        'find': "                query_unrepresentable: true,",
        'replace': "                query_unrepresentable: false,",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # Seven memories rebuilding to five vectors must not read as two lost.
        'name': "unrepresentable: stop reporting verdicts from a rebuild",
        'file': "src/store.rs",
        'find': "            unrepresentable: embedded.unrepresentable,",
        'replace': "            unrepresentable: 0,",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # The verdict note must actually be chosen for a verdict: its SQL asks for the empty row.
        'name': "unrepresentable: fail to recognise a verdict row",
        'file': "src/store.rs",
        'find': "\"SELECT 1 FROM vectors WHERE memory_id = ?1 AND length(vector) = 0\",",
        'replace': "\"SELECT 1 FROM vectors WHERE memory_id = ?1 AND length(vector) > 0\",",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # And the no-model note must survive for the no-model case: its remedy is the opposite one.
        'name': "unrepresentable: give every unfindable memory the verdict note",
        'file': "src/mcp.rs",
        'find': "            result[\"note\"] = json!(if unrepresentable {",
        'replace': "            result[\"note\"] = json!(if true {",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        'name': "unrepresentable: say nothing when the query cannot be represented",
        'file': "src/mcp.rs",
        'find': "        if coverage.query_unrepresentable {",
        'replace': "        if false {",
        'tests': ["unrepresentable"],
        'expect': "caught",
    },
    {
        # The whole log rests on one gate: a question nobody asked in a session is instrumentation,
        # and recording it as traffic would calibrate the floor against questions nobody asked.
        # Nothing about a logged row says which kind it was, so this cannot be noticed later.
        'name': "query log: record every recall, session or not",
        'file': "src/store.rs",
        'find': "        Asking::Silently => None,",
        'replace': "        Asking::Silently => Some((false, Timestamp::UNIX_EPOCH, SessionId::generate())),",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # The floor has to read the uncentred score, because centring removes the direction that
        # marks a query as out of domain. Logging the centred number instead would look entirely
        # healthy -- it is a similarity in the right range -- and would calibrate against the one
        # quantity measured not to work.
        'name': "query log: record the centred score as the raw one",
        'file': "src/store.rs",
        'find': "                .nearest_uncentred(&vector, 1)",
        'replace': "                .nearest_centred_on(&vector, 1, &centroid)",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # The scope label is the one a floor cannot suppress, and so the only thing that can lower a
        # floor set too high. Recorded wrongly, it would silently remove that correction.
        'name': "query log: invert the scope label",
        'file': "src/store.rs",
        'find': "                    model: model.clone(),\n                    scoped,",
        'replace': "                    model: model.clone(),\n                    scoped: !scoped,",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # A store's log is evidence about that store. Ids from the merged answer would credit one set
        # with memories it could never return, and a later use of one would label the wrong query.
        'name': "query log: log every hit to every store",
        'file': "src/store.rs",
        'find': "                .filter(|(_, at)| **at == index)",
        'replace': "                .filter(|(_, at)| **at != usize::MAX)",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # A silenced store is the one case where the log holds evidence the session never saw. Losing
        # the flag would leave a query indistinguishable from one that simply matched nothing.
        'name': "query log: never record a store as silenced",
        'file': "src/store.rs",
        'find': "                silenced: silenced[index],",
        'replace': "                silenced: false,",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # A store with no vectors scored nothing. Logging it with a default score would put a zero
        # into the very distribution the floor is read off, and it would be evidence about the
        # store's emptiness rather than about any query.
        'name': "query log: log a store that scored nothing",
        'file': "src/store.rs",
        'find': "            let (Some(raw_top), Some(centred_top)) = (raw_top[index], centred_top[index]) else {",
        'replace': "            let (raw_top, centred_top) = (\n                raw_top[index].unwrap_or(0.0),\n                centred_top[index].unwrap_or(0.0),\n            );\n            if false {",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # The session bound is what keeps the label meaning anything: without it, any later use of a
        # memory labels every query that ever returned it, the labelled set saturates, and the floor
        # calibrated from it sinks back toward the constant it replaced.
        'name': "query log: label a use from any session",
        'file': "src/store.rs",
        'find': "             WHERE session = ?1\n               AND id IN (SELECT query FROM query_log_hits WHERE memory_id = ?2)",
        'replace': "             WHERE (?1 IS NOT NULL)\n               AND id IN (SELECT query FROM query_log_hits WHERE memory_id = ?2)",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # And the label belongs to the queries that returned that memory, not to everything the
        # session happened to ask. Labelling the session wholesale would mark queries the set never
        # answered, which is exactly the evidence a floor is read off.
        'name': "query log: label every query the session asked",
        'file': "src/store.rs",
        'find': "               AND id IN (SELECT query FROM query_log_hits WHERE memory_id = ?2)",
        'replace': "               AND (?2 IS NOT NULL)",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # A row is written unlabelled and earns its label later. Writing it labelled would make every
        # query answerable, which is the floor reading its own output as evidence.
        'name': "query log: write every row already labelled",
        'file': "src/store.rs",
        'find': "                    used: false,",
        'replace': "                    used: true,",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # Three separate call sites produce the label, and each is a place to forget. One mutation
        # each, because losing any one of them loses that signal silently -- the log keeps filling,
        # and only the labelled count is smaller than it should be.
        'name': "query log: a link to a memory is not a use",
        'file': "src/mcp.rs",
        'find': "        let targets: Vec<MemoryId> = links.iter().map(|link| link.target.id()).collect();\n        self.note_used(&targets)?;",
        'replace': "        let targets: Vec<MemoryId> = links.iter().map(|link| link.target.id()).collect();\n        let _ = &targets;",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        'name': "query log: a memory passed back as already known is not a use",
        'file': "src/mcp.rs",
        'find': "        self.note_used(&already_known)?;",
        'replace': "        let _ = &already_known;",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        'name': "query log: looking a memory up is not a use",
        'file': "src/mcp.rs",
        'find': "        let reached: Vec<MemoryId> = hits.iter().map(|hit| hit.memory.id).collect();\n        self.note_used(&reached)?;",
        'replace': "        let reached: Vec<MemoryId> = hits.iter().map(|hit| hit.memory.id).collect();\n        let _ = &reached;",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # The session marker is the whole bound. A server outlives a session -- measured 2026-09-11,
        # two hours and two other sessions -- so without this roll the label reverts to the looser
        # process-shaped bound that the forty-query threshold was not measured under.
        'name': "query log: do not begin a new session when the conventions load",
        'file': "src/mcp.rs",
        'find': "        if params.cursor == 0 {\n            self.session = SessionId::generate();\n        }",
        'replace': "        if false {\n            self.session = SessionId::generate();\n        }",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # And paging is one load. Rolling on a continuation would split a session at the point the
        # catalog ran past its budget, dropping the label for everything asked before it finished.
        'name': "query log: begin a new session on every conventions page",
        'file': "src/mcp.rs",
        'find': "        if params.cursor == 0 {\n            self.session = SessionId::generate();",
        'replace': "        if params.cursor >= 0 {\n            self.session = SessionId::generate();",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # The floor is read against the raw score, and reading the centred one instead looks entirely
        # healthy -- a similarity in the right range -- while filtering on the quantity the
        # measurement ruled out: centred, the same stores silenced 2.7% of off-domain probes against
        # 62.7% raw.
        'name': "floor: filter on the centred score instead of the raw one",
        'file': "src/store.rs",
        'find': "                stores[*store]\n                    .vectors\n                    .raw_similarity(scored, &vector)",
        'replace': "                stores[*store]\n                    .vectors\n                    .raw_similarity(scored, &centroid)",
        'tests': ["recall"],
        'expect': "caught",
    },
    {
        # A floor belongs to one store. Applying one store's level across a multi-set recall would
        # silence a set that never calibrated anything, on a number read off somebody else's corpus.
        'name': "floor: give every store the first store's level",
        'file': "src/store.rs",
        'find': "                let Some(floor) = floors[*store] else {",
        'replace': "                let Some(floor) = floors[0] else {",
        'tests': ["floor", "cross_store"],
        'expect': "caught",
    },
    {
        # The margin and the threshold are both measured, and both fail quietly if changed: a floor at
        # the minimum silences the query that proved the set could answer, and one set from ten sits
        # too high and silences real questions with nothing able to contradict it.
        'name': "floor: calibrate with no margin below the lowest labelled score",
        'file': "src/store.rs",
        'find': "        let floor = percentile_of(&mut window, FLOOR_PERCENTILE) - FLOOR_MARGIN;",
        'replace': "        let floor = percentile_of(&mut window, FLOOR_PERCENTILE);",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        'name': "floor: set one from any number of labelled queries",
        'file': "src/store.rs",
        'find': "        if fresh < LABELS_PER_CALIBRATION {",
        'replace': "        if fresh < 1 {",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # Only a labelled query is evidence about where the floor belongs. Counting every query would
        # read the level off questions nothing says the set could answer, which is the case a floor
        # exists to silence.
        'name': "floor: calibrate from every query, labelled or not",
        'file': "src/store.rs",
        'find': "            \"SELECT id, raw_top FROM query_log WHERE used = 1 OR scoped = 1 ORDER BY id\",",
        'replace': "            \"SELECT id, raw_top FROM query_log ORDER BY id\",",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # A silenced query is the one case where the log holds evidence the session never saw, and the
        # only route by which a floor set too high can ever be corrected: nothing came back, so
        # nothing can be used, so only the scope label and the pre-floor score survive. Skipping the
        # row would make a bad floor permanent and silent.
        'name': "floor: do not log a query the floor silenced",
        'file': "src/store.rs",
        'find': "    if let Some((scoped, at, asked_by)) = session {",
        'replace': "    if let Some((scoped, at, asked_by)) = session.filter(|_| !silenced_by_floor) {",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # A floor calibrated under one model describes nothing under another: every set's floor moved
        # 0.10 to 0.16 under potion-multilingual-128M.
        'name': "floor: keep a level across a model change",
        'file': "src/store.rs",
        'find': "            self.clear_floor()?;",
        'replace': "            let _ = 0;",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # Silence and nothing-matched are the same empty result, and only the caller can tell which
        # deserved an answer. A recall that stopped reporting it would leave every silenced question
        # looking like an honest miss.
        'name': "floor: never report that a floor silenced everything",
        'file': "src/store.rs",
        'find': "        silenced_by_floor = hits.is_empty() && keep.iter().any(|kept| !kept);",
        'replace': "        silenced_by_floor = false;",
        'tests': ["floor", "mcp"],
        'expect': "caught",
    },
    {
        # And the way round it has to work, or the report is just news that the question failed.
        'name': "floor: ignore the caller's waiver",
        'file': "src/store.rs",
        'find': "    let floors: Vec<Option<f64>> = if question.ignore_floor {",
        'replace': "    let floors: Vec<Option<f64>> = if false {",
        'tests': ["floor", "mcp"],
        'expect': "caught",
    },
    {
        # The trigger counts labels the last calibration did not use. Counting every label instead
        # would fire on evidence already spent, recomputing the same level from the same queries
        # while the store moves underneath it.
        'name': "floor: count every label rather than the new ones",
        'file': "src/store.rs",
        # Re-indented 2026-09-24, when the loop moved into a block so the self-query floor could
        # borrow the store mutably after it.
        'find': "                if id > through {\n                    fresh += 1;\n                }",
        'replace': "                if id > -1 {\n                    fresh += 1;\n                }",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # The bound on the labelled set is what lets a level rise: read off everything a store was
        # ever asked, a percentile drifts toward the whole history instead of tracking the corpus.
        # Unbounded, the rule is the ratchet it replaced.
        'name': "floor: keep every labelled query for ever",
        'file': "src/store.rs",
        'find': "                params![labelled, i64::try_from(QUERY_LOG_KEPT).unwrap_or(i64::MAX)],",
        'replace': "                params![labelled, i64::MAX],",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # Pruning has to run where a label arrives, not only where a row is written: a row is written
        # unlabelled, so write-time pruning can bound only the unlabelled side.
        'name': "query log: prune only where rows are written",
        'file': "src/store.rs",
        'find': "        self.prune_query_log()?;\n        Ok(())\n    }",
        'replace': "        Ok(())\n    }",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # A substituted hit hands the caller two ids and the log records both, so that looking up the
        # superseded member -- which is how a session reads what was claimed before -- counts as a
        # use. Roughly 10% of returned hits are substitutions.
        'name': "query log: record only the tip of a substituted hit",
        'file': "src/store.rs",
        'find': "                .flat_map(|(hit, _)| std::iter::once(hit.memory.id).chain(hit.matched))",
        'replace': "                .flat_map(|(hit, _)| std::iter::once(hit.memory.id))",
        'tests': ["query_log"],
        'expect': "caught",
    },
    {
        # The whole entry is this comparison. A client serves whatever binary it launched with, and
        # the failure is silent in the way that matters most: a catalog a build behind answers every
        # call successfully and governs the session's reasoning. Three occurrences recorded.
        'name': "stale server: never report that the binary was rebuilt",
        'file': "src/mcp.rs",
        'find': "        (now != *launched).then_some(now)",
        'replace': "        let _ = launched;\n        None",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # And the other end of it: reporting a rebuild that did not happen would teach a session to
        # ignore the notice, which costs exactly the occasions it exists for.
        'name': "stale server: report a rebuild on every call",
        'file': "src/mcp.rs",
        'find': "        (now != *launched).then_some(now)",
        'replace': "        let _ = launched;\n        Some(now)",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The guard that separates a summary saying what a set is *about* from one saying what it is
        # *called*. A keyword on every memory -- the project's own name, most often -- discriminates
        # nothing, and admitting it would fill every cluster's label with the same word. Zero is not
        # an arbitrary threshold: it is the point where a keyword stops being commoner inside its
        # cluster than outside it.
        'name': "set summary: admit keywords that discriminate nothing",
        'file': "src/summary.rs",
        'find': "            if score > 0.0 {",
        'replace': "            if score >= 0.0 {",
        'tests': ["summary"],
        'expect': "caught",
    },
    {
        # Counting raw spellings splits one idea into two half-strength rivals that each lose to a
        # weaker keyword, and nothing reports the split -- the same failure `keyword-standards` names
        # for coined terms, one layer down in the machinery.
        'name': "set summary: count keyword spellings rather than the caseless key",
        'file': "src/summary.rs",
        'find': "                seen.entry(keyword.folded()).or_insert(keyword);",
        'replace': "                seen.entry(keyword.as_str().to_owned()).or_insert(keyword);",
        'tests': ["summary"],
        'expect': "caught",
    },
    {
        # Smoothing has to act on the proportions, not on a raw count. A floor of half a memory means
        # one thing in a corpus of eight hundred and something else entirely in a corpus of four --
        # the constant-count form scored a genuinely distinctive keyword negative on a three-memory
        # cluster, which is how this constant came to be where it is.
        'name': "set summary: smooth the counts instead of the proportions",
        'file': "src/summary.rs",
        'find': "            let p_outside = (outside as f64 + SMOOTHING) / (rest as f64 + 2.0 * SMOOTHING);",
        'replace': "            let p_outside = (outside as f64).max(SMOOTHING) / (rest.max(1) as f64);",
        'tests': ["summary"],
        'expect': "caught",
    },
    {
        # The defect `clusters/2` fixed, put back: each memory's seeding draw keyed by its position
        # rather than by its id. Inserting a memory then shifts every later memory's draw and
        # regroups most of the set -- `a-set-description-reshuffles-on-a-small-change`.
        'name': "set summary: key seeding draws by position rather than by memory",
        'file': "src/summary.rs",
        'find': "                    let bits = fnv(\n                        subject.key,",
        'replace': "                    let bits = fnv(\n                        index as u64,",
        'tests': ["summary"],
        'expect': "caught",
    },
    {
        # Keeping the first seeding instead of the most cohesive of ten. What restarts buy is
        # statistical -- the worst agreement after removing one memory rose from 0.57 to 0.98 on
        # `coding-lessons` -- and no fixture small enough to be a unit test has near-equal optima to
        # choose between.
        'name': "set summary: keep the first seeding rather than the most cohesive",
        'file': "src/summary.rs",
        'find': "            Some((held, _)) => cohesion > *held,",
        'replace': "            Some(_) => false,",
        'tests': ["summary"],
        'expect': "survives",
        'reason': "The benefit of restarts is stability under a one-memory change on a real corpus, "
                  "measured by `probe_how_stable_a_set_description_is` in tests/probes.rs. A unit "
                  "fixture has no near-equal optima for the choice to matter between, and one "
                  "contrived to have them would pin an accident of its data rather than the rule.",
    },
    {
        # Identity records are never embedded, so counting them makes every set permanently
        # incomplete: no import could ever describe itself, and every one would owe a description.
        # **Re-aimed 2026-09-22**, when the filter moved into `gaps`, the one query the sweep and
        # `awaiting_vectors` now share: the sweep no longer finds an identity record on every call.
        'name': "set description: count identity records as awaiting vectors",
        'file': "src/store.rs",
        'find': "             WHERE v.memory_id IS NULL AND m.created_by <> m.id",
        'replace': "             WHERE v.memory_id IS NULL",
        'tests': ["set_description"],
        'expect': "caught",
    },
    {
        # One set answering to two identities on two machines was silent until 2026-09-22
        # (`an-import-silently-keeps-a-different-set-identity`), then reported and imported anyway;
        # since 2026-09-26 it is refused (`identity-mismatch-refusal`).
        'name': "set identity: import a file declaring another identity",
        'file': "src/mcp.rs",
        'find': "            && declared != held\n        {\n            return Err(identity_mismatch(",
        'replace': "            && false && declared != held\n        {\n            return Err(identity_mismatch(",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A file with records and no identity is the first step of the hand-renamed fork's trap,
        # and the only moment it can be caught before copies diverge.
        'name': "set identity: say nothing when a file with records declares none",
        'file': "src/mcp.rs",
        'find': "            (None, Some(held)) if has_records => Some(format!(",
        'replace': "            (None, Some(held)) if false && has_records => Some(format!(",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The other place both identities meet: an export rewriting the committed identity, which a
        # diff shows only as a changed header line. Refused since 2026-09-26.
        'name': "set identity: export over a file declaring another identity",
        'file': "src/mcp.rs",
        'find': "        (*declared != writing)\n            .then(|| identity_mismatch(",
        'replace': "        false\n            .then(|| identity_mismatch(",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The dropped-records refusal advises importing from the destination, which a mismatch
        # refuses too; asking identity first is what keeps its advice from leading nowhere.
        'name': "set identity: refuse dropped records ahead of a mismatch",
        'file': "src/mcp.rs",
        'find': "        if let Some(refusal) = replaced.identity_refusal(&path, scope, identity) {\n            return Err(refusal);\n        }\n        if let Some(refusal) = replaced.refusal(&path) {\n            return Err(refusal);\n        }",
        'replace': "        if let Some(refusal) = replaced.refusal(&path) {\n            return Err(refusal);\n        }\n        if let Some(refusal) = replaced.identity_refusal(&path, scope, identity) {\n            return Err(refusal);\n        }",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The reverse of the rule this replaced: an import no longer displaces an empty set's identity, so
        # leaving it undefended lets an export overwrite another set's file with a rival.
        'name': 'set identity: stop defending the identity an empty set holds',
        'file': 'src/mcp.rs',
        'find': 'replaced.identity_refusal(&path, scope, identity)',
        'replace': 'replaced.identity_refusal(&path, scope, identity.filter(|_| store.holds_any_record().unwrap_or(true)))',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A write embeds its own memory and needs no other memory's vector, so the sweep runs before
        # a search by meaning and not on a write (`where-the-gap-sweep-runs`).
        'name': "gap sweep: fill every gap on a write again",
        'file': "src/store.rs",
        'find': '        self.mint_set_id()?;\n        if self.would_destroy_an_unrebuildable_vector(memory)? {',
        'replace': '        self.prepare()?;\n        self.mint_set_id()?;\n        if self.would_destroy_an_unrebuildable_vector(memory)? {',
        'tests': ["gap_sweep"],
        'expect': "caught",
    },
    {
        # The hole the owed description closes: an import that cannot describe what it brought
        # records nothing, and installing the model afterwards evades the gate entirely.
        'name': "set description: skip the description an import could not give",
        'file': "src/mcp.rs",
        'find': "        let (describes, description_owed) = if awaiting > 0 {",
        'replace': "        let (describes, description_owed) = if false && awaiting > 0 {",
        'tests': ["set_description"],
        'expect': "caught",
    },
    {
        # A continuation page is the same load; repeating the report there makes it the loudest
        # thing in the catalog.
        'name': "set description: pay owed descriptions on every conventions page",
        'file': "src/mcp.rs",
        'find': "        let paid = if params.cursor == 0 {",
        'replace': "        let paid = if true {",
        'tests': ["set_description"],
        'expect': "caught",
    },
    {
        # While memories still lack vectors there is nothing to say. Reporting anyway would describe
        # a set with the import left out -- the partial description the deferral exists to avoid.
        'name': "set description: report while memories still lack vectors",
        'file': "src/mcp.rs",
        'find': "            .map_err(|error| format!(\"could not tell whether the set is embedded: {error}\"))?\n            > 0",
        'replace': "            .map_err(|error| format!(\"could not tell whether the set is embedded: {error}\"))?\n            > usize::MAX",
        'tests': ["set_description"],
        'expect': "caught",
    },


    {
        # Shown once and dropped, a session that skimmed past the report has discharged it. The
        # decision was that only an export -- which shows the description itself -- discharges it.
        'name': "set description: discharge the owed description the first time it is reported",
        'file': "src/mcp.rs",
        'find': "        let derived = Self::derive_summary(store)?;\n        Ok(Some(json!({",
        'replace': "        store.discharge_description().map_err(|error| error.to_string())?;\n        let derived = Self::derive_summary(store)?;\n        Ok(Some(json!({",
        'tests': ["set_description"],
        'expect': "caught",
    },    {
        # A file an older build wrote carries a `summary` in its header. Refusing unknown header
        # fields would make every such file unreadable, for a field nothing reads any more.
        'name': "set description: refuse a file whose header still carries a description",
        'file': "src/memory_file.rs",
        'find': "#[derive(Debug, Serialize, Deserialize)]\npub struct Header {",
        'replace': "#[derive(Debug, Serialize, Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct Header {",
        'tests': ["set_description"],
        'expect': "caught",
    },
    {
        # The note is the only thing in an import response that says the description is owed
        # rather than simply absent.
        'name': "set description: drop the note that the description is owed",
        'file': "src/mcp.rs",
        'find': "        if let Some(note) = description_owed {",
        'replace': "        if let Some(note) = description_owed.filter(|_| false) {",
        'tests': ["set_description"],
        'expect': "caught",
    },

    {
        # An export with no model, or a mismatched one, shows none or part of the set; discharging
        # there retires the report without the description ever having been shown.
        'name': "set description: discharge at an export that described only part of the set",
        'file': "src/mcp.rs",
        'find': "        if awaiting == 0 {",
        'replace': "        if true {",
        'tests': ["set_description"],
        'expect': "caught",
    },
    {
        # Without the catch-up, an export made straight after the model arrives describes nothing,
        # and the report it should discharge stands for another cycle.
        'name': "set description: export without embedding what is missing first",
        'file': "src/mcp.rs",
        'find': "            .catch_up_vectors()\n            .map_err(|error| format!(\"could not embed the set before describing it: {error}\"))?;",
        'replace': "            .awaiting_vectors()\n            .map_err(|error| format!(\"could not embed the set before describing it: {error}\"))?;",
        'tests': ["set_description"],
        'expect': "caught",
    },


    {
        # Every reserved mark on a charter is a `hoard-` keyword, starting with the charter mark
        # itself, so matching the family rather than `hoard-lang-` reports `scope-topics` as a
        # language the set is written in.
        'name': "set languages: read any reserved keyword on a charter as a language",
        'file': "src/store.rs",
        'find': "                if let Some(tag) = keyword.folded().strip_prefix(LANGUAGE_KEYWORD_PREFIX)",
        'replace': "                if let Some(tag) = keyword.folded().strip_prefix(\"hoard-\")",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # Keywords keep their author's spelling and match without case, so reading the spelling
        # reports `zh-Hans` and `zh-hans` as two declarations of one language.
        'name': "set languages: read a tag in the spelling it was written with",
        'file': "src/store.rs",
        'find': "                if let Some(tag) = keyword.folded().strip_prefix(LANGUAGE_KEYWORD_PREFIX)",
        'replace': "                if let Some(tag) = keyword.as_str().strip_prefix(LANGUAGE_KEYWORD_PREFIX)",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # Nothing yet stops a second live charter, so reading only one hides what the other declares.
        'name': "set languages: read only the first live charter",
        'file': "src/store.rs",
        'find': "        for charter in &charters {",
        'replace': "        for charter in charters.iter().take(1) {",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # A set with no charter told to add a keyword to its charter is sent looking for a memory
        # that does not exist. The two absences need two notes.
        'name': "set languages: treat a set with no charter as one whose charter declares nothing",
        'file': "src/store.rs",
        'find': "        if charters.is_empty() {\n            return Ok(None);",
        'replace': "        if false {\n            return Ok(None);",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # The prompt for every existing set, `local` above all: `local` is never made through
        # `create`, so without this nothing ever asks it to declare a language.
        'name': "set languages: say nothing about a charter that declares no language",
        'file': "src/mcp.rs",
        'find': "                Some(tags) if tags.is_empty() => {",
        'replace': "                Some(tags) if false && tags.is_empty() => {",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # Restore writes a file's records by its own path, past the claim in `write`, and every record is
        # canonicalised against the identity of the set holding it.
        'name': 'set identity: let a restore write records with no identity claimed',
        'file': 'src/store.rs',
        'find': '        // Before any record: each is canonicalised against the identity of the set holding it.\n        self.mint_set_id()?;\n',
        'replace': '',
        'tests': ['store'],
        'expect': "caught",
    },
    {
        # Claimed before reconciling, not at the first memory: tombstones go through `accept_tombstone`,
        # which does not mint, so a file of tombstones alone would leave records with no identity.
        'name': "set identity: leave an import's identity to its first memory",
        'file': 'src/mcp.rs',
        'find': '        let held = if validated.ids.is_empty() {',
        'replace': '        let held = if true {',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # An empty file is not a set's first write. Settling an identity from one fixes the set under an
        # identity nobody else holds before the file carrying its own arrives.
        'name': 'set identity: let an empty file settle an identity',
        'file': 'src/mcp.rs',
        'find': '        let held = if validated.ids.is_empty() {',
        'replace': '        let held = if false {',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A set with no identity meeting a file that declares one skipped the import that would give it
        # the file's; writing over the file forks every copy taken from it.
        'name': 'set identity: let a set with no identity write over a file that declares one',
        'file': 'src/mcp.rs',
        'find': '        if identity.is_none()\n            && let Some(declared) = replaced.declared()',
        'replace': '        if false\n            && let Some(declared) = replaced.declared()',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # An export proposes an identity for a set with none; the file carries it, so the set has to hold it
        # too, or the first copy taken from the file is a different set.
        'name': 'set identity: never keep the identity an export proposed',
        'file': 'src/mcp.rs',
        'find': '        if identity.is_none() {\n            let adopted = store',
        'replace': '        if false {\n            let adopted = store',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # Minting the author record can be the set's first write, which is what gives it an identity, so the
        # identity read before it is stale and the durable map row would go unwritten.
        'name': 'author map: key the row on the identity read before minting',
        'file': 'src/mcp.rs',
        'find': '                let set_id = self\n                    .stores\n                    .get(scope)\n                    .ok_or_else(|| missing_scope(&self.withheld, scope))?\n                    .set_id()\n                    .map_err(|error| format!("could not read the set\'s identity: {error}"))?;\n                if let Some(set_id) = set_id {\n                    self.remember_author(scope, set_id, record)?;',
        'replace': '                if let Some(set_id) = set_id {\n                    self.remember_author(scope, set_id, record)?;',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # An import that mints an identity nobody else holds has to say so: every machine importing that
        # file does the same, and their copies are different sets.
        'name': 'set identity: stay silent when an import mints an identity of its own',
        'file': 'src/mcp.rs',
        'find': '            (None, Some(held)) if has_records && minted_now => Some(format!(',
        'replace': '            (None, Some(held)) if false && minted_now => Some(format!(',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # The rule itself: which of the two a stored link is, decided by name alone again. A link
        # naming its own set by an old name is stored as cross-set and supersession ignores it.
        'name': "link identity: decide same-set by name even where both sides have an identity",
        'file': "src/memory/link.rs",
        'find': "            } if holder.is_some() => holder != Some(*target),",
        'replace': "            } if false => holder != Some(*target),",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # The one step every write path shares. Passing no identity makes every path name-only.
        'name': "link identity: canonicalise links without the holding set's identity",
        'file': "src/store.rs",
        'find': "            } if link.target.is_foreign_to(scope, holder) => (",
        'replace': "            } if link.target.is_foreign_to(scope, None) => (",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # The ordinary write, which `insert` and import both reach.
        'name': "link identity: write a memory without the holding set's identity",
        'file': "src/store.rs",
        'find': "        if let Some(memory) = memory {\n            write_memory(&transaction, memory, &self.scope, holder)?;",
        'replace': "        if let Some(memory) = memory {\n            write_memory(&transaction, memory, &self.scope, None)?;",
        'tests': ["cross_store"],
        'expect': "caught",
    },
    {
        # Restore rewrites the whole set from a file, memories first.
        'name': "link identity: restore memories without the holding set's identity",
        'file': "src/store.rs",
        'find': "                    // the way in.\n                    write_memory(&transaction, memory, &self.scope, holder)?;",
        'replace': "                    // the way in.\n                    write_memory(&transaction, memory, &self.scope, None)?;",
        'tests': ["cross_store", "sets"],
        'expect': "caught",
    },
    {
        # Restore's tombstones keep their ancestry as edges, written through the same step.
        'name': "link identity: restore a tombstone's ancestry without the holding set's identity",
        'file': "src/store.rs",
        'find': "                        &self.scope,\n                        holder,\n                    )?;",
        'replace': "                        &self.scope,\n                        None,\n                    )?;",
        'tests': ["cross_store", "sets"],
        'expect': "caught",
    },
    {
        # `entomb` writes a tombstone's ancestry. A memory forgotten on this machine cannot reach it
        # in the long spelling -- its ancestry is read back from rows already canonicalised -- but a
        # tombstone arriving from a file through `accept_tombstone` carries its ancestry as written.
        'name': "link identity: keep a forgotten memory's ancestry without the holding set's identity",
        'file': "src/store.rs",
        'find': "            &self.scope,\n            holder,\n        )?;",
        'replace': "            &self.scope,\n            None,\n        )?;",
        'tests': ["cross_store", "store"],
        'expect': "caught",
    },
    {
        # Every attached client launches the binary bare. A bare invocation that did anything but
        # serve would take every client down at once, reported as a crashed server.
        'name': "cli: stop serving on a bare invocation",
        'file': "src/cli.rs",
        'find': "        return Invocation::Serve;",
        'replace': "        return Invocation::Help;",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # Ignoring an unrecognised argument starts a server waiting on stdin: a hang, from a shell.
        'name': "cli: serve on an unknown command instead of refusing it",
        'file': "src/cli.rs",
        'find': "        _ => Invocation::Refused(format!(\"unknown command {first:?}\")),",
        'replace': "        _ => Invocation::Serve,",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # `scan` with nothing to scan would succeed having looked at nothing.
        'name': "cli: accept scan with no files",
        'file': "src/cli.rs",
        'find': "            if files.is_empty() {",
        'replace': "            if false {",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # A hook scans the staged blob through standard input; `-` read as a path fails to open.
        'name': "cli: treat `-` as a file name",
        'file': "src/cli.rs",
        'find': "        let (name, text) = if file == \"-\" {",
        'replace': "        let (name, text) = if false {",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # The line number is how a finding is found without reproducing it.
        'name': "cli: number lines from zero",
        'file': "src/cli.rs",
        'find': "                    index + 1,",
        'replace': "                    index,",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # Hook output lands in terminals and CI logs; printing the line prints the credential.
        'name': "cli: print the line a finding is on",
        'file': "src/cli.rs",
        'find': "                    memory_named_by(line)",
        'replace': "                    line",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # A finding that exits zero lets the commit through, which is the whole of the failure.
        'name': "cli: exit zero on a finding",
        'file': "src/cli.rs",
        'find': "        FOUND\n",
        'replace': "        0\n",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # A file that could not be read is not a clean file.
        'name': "cli: report an unreadable file as clean",
        'file': "src/cli.rs",
        'find': "    if unreadable {",
        'replace': "    if false && unreadable {",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # A record whose content differs from the file's copy is a change the reader has to see --
        # another session's amendment, or a forgetting.
        'name': "replaced file: count a changed record as unchanged",
        'file': "src/mcp.rs",
        'find': "                Some(before) if before == *checksum => unchanged += 1,",
        'replace': "                Some(_) => unchanged += 1,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The case most likely to cost work: records the file held and this export would overwrite.
        # Nothing removed means nothing refused, so the refusal tests catch this.
        'name': "replaced file: find nothing removed",
        'file': "src/mcp.rs",
        'find': "        let mut removed: Vec<MemoryId> = held.into_keys().collect();",
        'replace': "        let mut removed: Vec<MemoryId> = Vec::new();",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # No file and an unreadable file want opposite handling: a first export must not be refused.
        'name': "replaced file: treat a missing file as unreadable",
        'file': "src/mcp.rs",
        'find': "            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {\n                return Self::Absent {",
        'replace': "            Err(error) if false && error.kind() == std::io::ErrorKind::NotFound => {\n                return Self::Absent {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # One format change can alter every record; an uncapped list buries the report.
        'name': "replaced file: list every id in a category",
        'file': "src/mcp.rs",
        'find': "            .take(REPLACED_IDS_LISTED)\n",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The refusal itself: an export writes over records the file holds and the set does not.
        'name': "replaced file: skip the refusal before writing",
        'file': "src/mcp.rs",
        'find': "        if let Some(refusal) = replaced.refusal(&path) {",
        'replace': "        if let Some(refusal) = None::<String> {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A file that cannot be read -- a merge left with conflict markers, most likely -- written
        # over as though nothing were there, losing the other side of the merge.
        'name': "replaced file: write freely over a file that did not parse",
        'file': "src/mcp.rs",
        'find': "            Ok(header) => header.set_id,\n            Err(error) => return Self::Unreadable(error.to_string()),",
        'replace': "            Ok(header) => header.set_id,\n            Err(_) => return Self::Absent { written: written.len() },",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A removal refused with nothing to name. Importing first is the remedy, so the export has to
        # go through once the set holds the file's records -- this refuses it regardless.
        'name': "replaced file: refuse an export that removes nothing",
        'file': "src/mcp.rs",
        'find': "            Self::Compared { removed, .. } if removed.is_empty() => None,",
        'replace': "            Self::Compared { removed, .. } if removed.is_empty() && false => None,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The block-list itself: a typo or an invented `hoard-` mark goes into the set unremarked.
        'name': "control marks: skip the known-mark check on an authored write",
        'file': "src/store.rs",
        'find': "        self.check_control_marks_known(memory)?;",
        'replace': "",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The set vouches for its own terms. Without it a retired mark strands the memories carrying
        # it, and a mark from a newer version cannot be written again by an older one.
        'name': "control marks: stop counting a term the set already holds as known",
        'file': "src/store.rs",
        'find': "            if self.holds_keyword(&folded)? {",
        'replace': "            if false && self.holds_keyword(&folded)? {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # A bare family prefix names no language and no project mark.
        'name': "control marks: accept a family prefix with nothing after it",
        'file': "src/store.rs",
        'find': "                    .is_some_and(|member| !member.is_empty())",
        'replace': "                    .is_some()",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # Keywords match caselessly, and the list is spelled folded: comparing the written spelling
        # refuses a known mark written in capitals.
        'name': "control marks: compare the written spelling against the list",
        'file': "src/store.rs",
        'find': "            if CONTROL_MARKS.contains(&folded.as_str()) || in_a_family {",
        'replace': "            if CONTROL_MARKS.contains(&keyword.as_str()) || in_a_family {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The list and the served catalog are held together: a mark the catalog tells a session to
        # write, dropped from the list, would be refused on every new set.
        'name': "control marks: drop a served watch-list mark from the list",
        'file': "src/store.rs",
        'find': "    COLLECT_MARK,\n",
        'replace': "",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        # The guard itself. Memories about charters were written carrying the charter mark three
        # times in six days, each in a set that already had one, and served text did not stop it.
        'name': "charter guard: skip the one-live-charter check on an authored write",
        'file': "src/store.rs",
        'find': "        self.check_one_live_charter(memory)?;",
        'replace': "",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # Superseding the charter is how a charter changes. Counting the charter being replaced as
        # still standing would refuse the one legitimate way to write a new one.
        'name': "charter guard: count a charter the write supersedes as still standing",
        'file': "src/store.rs",
        'find': "            if charter != memory.id && !replaced.contains(&charter) {",
        'replace': "            if charter != memory.id {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # An amendment rewrites a memory under its own id, so the live charter amended in place
        # would be refused as a second charter beside itself.
        'name': "charter guard: count the memory being written as a charter beside itself",
        'file': "src/store.rs",
        'find': "            if charter != memory.id && !replaced.contains(&charter) {",
        'replace': "            if !replaced.contains(&charter) {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The guard reaches only memories carrying the mark. Applying it to every write would refuse
        # the memory about charters that the refusal itself tells the caller to write.
        'name': "charter guard: treat every memory as carrying the charter mark",
        'file': "src/store.rs",
        'find': "            .any(|keyword| keyword.folded() == CHARTER_KEYWORD)",
        'replace': "            .any(|_| true)",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # Absorbing can leave two live charters, and then supersedes this set's charter if it linked
        # into the absorbed set. Through `insert`, that replacement is refused as a second charter and
        # the absorb stops with the records already merged.
        'name': "charter guard: refuse a charter an absorb carries forward",
        'file': "src/mcp.rs",
        'find': "            store.carry_forward(&replacement).map_err(|error| {",
        'replace': "            store.insert(&replacement).map_err(|error| {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The defect itself: with the set out of the key, two links to one id in different sets
        # collide and `INSERT OR REPLACE` keeps the second. Silent -- the write returns Ok.
        #
        # **Survives, and the reason is a second route rather than a missing test.** `CREATE TABLE
        # IF NOT EXISTS` builds the table, and then `rekey_links_by_target_set` runs at every open,
        # sees a three-column key and rebuilds it to four -- so a store opened from mutated schema
        # text is repaired before any test can look at it, including a brand new in-memory one.
        # Narrowing this line alone therefore cannot be observed through the public API at all. The
        # entry below mutates the migration's own CREATE TABLE, which is the half that can be.
        'name': "links: drop the target set from the row key",
        'file': "src/store.rs",
        'find': "                 PRIMARY KEY (source, label, target, target_set)\n             );\n             -- The reverse direction of the same rows.",
        'replace': "                 PRIMARY KEY (source, label, target)\n             );\n             -- The reverse direction of the same rows.",
        'tests': ["store"],
        'expect': "survives",
    },
    {
        # The migration is the reachable half of the same guard: rebuild to the old narrow key and
        # an existing store stays on it for ever, re-migrating at every open and never widening.
        'name': "links rekey: rebuild onto the narrow key",
        'file': "src/store.rs",
        'find': "                 PRIMARY KEY (source, label, target, target_set)\n             );\n             INSERT INTO links_rekeyed",
        'replace': "                 PRIMARY KEY (source, label, target)\n             );\n             INSERT INTO links_rekeyed",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The other half. SQLite holds NULLs in a key distinct, and a same-set link is a NULL
        # `target_set`, so without this index the wider key stops collapsing two identical same-set
        # links -- a silent duplicate where the defect was a silent loss.
        'name': "links: stop collapsing identical same-set links",
        'file': "src/store.rs",
        'find': "             CREATE UNIQUE INDEX IF NOT EXISTS links_same_set\n                 ON links(source, label, target) WHERE target_set IS NULL;\n             -- Vectors are derived state",
        'replace': "             -- Vectors are derived state",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # Narrowing the long spelling onto an existing compact row violates `links_same_set`, and
        # this runs at *open*: a plain UPDATE takes the whole store down rather than failing a write.
        'name': "links: narrow own-set links without replacing",
        'file': "src/store.rs",
        'find': "            \"UPDATE OR REPLACE links SET target_set = NULL, target_keywords = NULL \\",
        'replace': "            \"UPDATE links SET target_set = NULL, target_keywords = NULL \\",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The migration's detection. Reading the old key as already-migrated leaves an existing
        # store on the narrow key for ever, where the defect stays live and nothing reports it.
        'name': "links rekey: treat the old key as already migrated",
        'file': "src/store.rs",
        'find': "        if key_columns >= 4 {",
        'replace': "        if key_columns >= 3 {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The two destination forms are not interchangeable: a directory gets the well-known path
        # appended, a file is written exactly as given. Swapping them writes a backup somewhere the
        # caller did not ask for -- and, for the file form, somewhere auto-discovery will find it.
        'name': "export subcommand: write the file form under the well-known path",
        'file': "src/cli.rs",
        'find': "        Some(\"--directory\") => Invocation::Export {\n            scope,\n            destination: Destination::Directory(PathBuf::from(path)),",
        'replace': "        Some(\"--directory\") => Invocation::Export {\n            scope,\n            destination: Destination::File(PathBuf::from(path)),",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # A scheduled backup is watched by its exit status and read by nobody, so a refusal that
        # exits zero is a backup that silently stopped happening -- the failure the subcommand was
        # built to make visible.
        'name': "export subcommand: exit zero when the export was refused",
        'file': "src/main.rs",
        'find': "                Err(message) => {\n                    eprintln!(\"scatterhoard: {message}\");\n                    std::process::ExitCode::from(cli::COULD_NOT_RUN)",
        'replace': "                Err(message) => {\n                    eprintln!(\"scatterhoard: {message}\");\n                    std::process::ExitCode::SUCCESS",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # The header cannot be recomputed without the set's vectors, so the receiving side's is
        # carried over. Taking theirs would hand the merged file a summary describing a set state
        # that never existed on this side -- and nothing downstream would report it.
        'name': "merge: carry over their header instead of ours",
        'file': "src/merge.rs",
        'find': "    let header = header_line(ours)?;",
        'replace': "    let header = header_line(theirs)?;",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # `created` never moves, so a disagreement is corruption rather than an edit. Merging past it
        # buries exactly the thing worth stopping for.
        'name': "merge: merge past a disagreement about created",
        'file': "src/merge.rs",
        'find': "    if our_created != theirs.created {\n        return None;\n    }",
        'replace': "    if false {\n        return None;\n    }",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # The forced decision: any edit moves `modified`, so the two sides always disagree there and
        # the later one wins. Taking the earlier silently rolls an attribution backwards.
        'name': "merge: take the earlier modified rather than the later",
        'file': "src/merge.rs",
        'find': "    let later = if theirs.modified.at > our_modified.at {",
        'replace': "    let later = if theirs.modified.at < our_modified.at {",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # The three-way scalar decision. Returning our own value where *they* changed the field
        # discards their edit while still reporting a clean merge, which is the silent half of the
        # failure the driver exists to prevent.
        'name': "merge: drop the other side's scalar change",
        'file': "src/merge.rs",
        'find': "        Some(base) if base == ours => Some(theirs.clone()),",
        'replace': "        Some(base) if base == ours => Some(ours.clone()),",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # `&&` is what makes this a three-way set merge rather than a union: an item either side
        # removed stays removed. `||` resurrects a keyword somebody deliberately retired.
        'name': "merge: union collections instead of set-merging them",
        'file': "src/merge.rs",
        'find': "        .filter(|item| ours.contains(item) && theirs.contains(item))",
        'replace': "        .filter(|item| ours.contains(item) || theirs.contains(item))",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # `git` learns a merge failed from the exit status alone. Exiting zero on a divergence tells
        # it the conflicted file is the final answer, and the receiving side's version is committed.
        'name': "merge driver: exit zero even when records diverged",
        'file': "src/cli.rs",
        'find': "    if outcome.conflicts.is_empty() {\n        return 0;\n    }",
        'replace': "    if true {\n        return 0;\n    }",
        'tests': ["cli"],
        'expect': "caught",
    },
    # `file-position-column`, 2026-09-22: each row stores its file position so the database orders
    # an export, and every position streamed is checked against its id.
    {
        # Stored unsigned-as-signed, a hash with its top bit set sorts first; the database's order
        # then differs from the file's.
        'name': "store: store the file position without flipping its sign",
        'file': "src/store.rs",
        'find': "    i64::from_ne_bytes((hash ^ (1 << 63)).to_ne_bytes())",
        'replace': "    i64::from_ne_bytes(hash.to_ne_bytes())",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # A position that drifted but kept the order is found only by comparing it with its id.
        'name': "store: trust a stored position without checking it against its id",
        'file': "src/store.rs",
        'find': "            if stored != Some(stored_position(id)) || previous.is_some_and(|last| last >= position)",
        'replace': "            if previous.is_some_and(|last| last >= position)",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # An id held as a memory and a tombstone carries one correct position twice.
        'name': "store: stream a record that repeats the one before it",
        'file': "src/store.rs",
        'find': "            if stored != Some(stored_position(id)) || previous.is_some_and(|last| last >= position)",
        'replace': "            if stored != Some(stored_position(id))",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "store: skip the file-position backfill on opening an older store",
        'file': "src/store.rs",
        'find': "            if !missing.is_empty() {",
        'replace': "            if false {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "store: write a memory with no file position",
        'file': "src/store.rs",
        'find': "            stored_position(memory.id),\n",
        'replace': "            None::<i64>,\n",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "store: forget a memory with no file position on its tombstone",
        'file': "src/store.rs",
        'find': "                stored_position(id),\n            ],",
        'replace': "                None::<i64>,\n            ],",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The import path writes tombstones too, and a file's tombstone is the likelier arrival.
        'name': "store: import a tombstone with no file position",
        'file': "src/store.rs",
        'find': "                            stored_position(tombstone.id),\n",
        'replace': "                            None::<i64>,\n",
        'tests': ["sets"],
        'expect': "caught",
    },
    # `streaming-merge-driver`, 2026-09-22: sides in file order are walked in step, one record a
    # side; a side out of order sends the merge down the whole-file path, and is named.
    {
        # Pass one is the only thing that knows a side is out of order. Missing it walks an id-ordered
        # base in step, which passes its records by unpaired.
        'name': "merge: never notice a side out of file order",
        'file': "src/merge.rs",
        'find': "            in_order = false;\n",
        'replace': "",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        'name': "merge: walk in step even with a side out of file order",
        'file': "src/merge.rs",
        'find': "    let conflicts = if out_of_order.is_some() {",
        'replace': "    let conflicts = if false {",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # The next record to write is the lower position; the higher skips the other side's record,
        # which is then never written.
        'name': "merge: step to the higher of ours and theirs",
        'file': "src/merge.rs",
        'find': "our_position.min(their_position)",
        'replace': "our_position.max(their_position)",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # A base left behind pairs later records with no ancestor, and compatible edits conflict.
        'name': "merge: never advance the base",
        'file': "src/merge.rs",
        'find': "        base.skip_before(next)?;\n",
        'replace': "",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        'name': "merge: advance the base past the record it should stop on",
        'file': "src/merge.rs",
        'find': "        while self.position().is_some_and(|here| here < position) {",
        'replace': "        while self.position().is_some_and(|here| here <= position) {",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # The order proved on the first read is relied on in the second; a side that changed between
        # them would be mispaired silently rather than refused.
        'name': "merge: trust the second read's order",
        'file': "src/merge.rs",
        'find': "                if self.last.is_some_and(|last| last >= position) {",
        'replace': "                if false {",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # A leading blank line copied as the header writes a file with no header at all.
        'name': "merge: take a blank line as the header",
        'file': "src/merge.rs",
        'find': "        if !line.trim().is_empty() {\n            return Ok(line);",
        'replace': "        if true {\n            return Ok(line);",
        'tests': ["merge"],
        'expect': "caught",
    },
    {
        # The one path that holds whole sets; a person with a slow merge learns nothing without it.
        'name': "merge driver: say nothing about a side out of file order",
        'file': "src/cli.rs",
        'find': "    if let Some(side) = outcome.out_of_order {",
        'replace': "    if let Some(side) = None::<&str> {",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # `drop` is the only tool that reaches a withheld store; without it the remedy is deleting a
        # database by hand, with no safety copy.
        'name': "drop: refuse a withheld set as not loaded",
        'file': "src/mcp.rs",
        'find': "            .or_else(|| self.withheld.get(&scope))",
        'replace': "            .or_else(|| None)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A withheld set answering "not loaded" with no reason reads as a lost set.
        'name': "missing_scope: report a withheld set as merely unknown",
        'file': "src/mcp.rs",
        'find': "    if withheld.contains_key(scope) {\n        return withheld_scope(scope);",
        'replace': "    if false {\n        return withheld_scope(scope);",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The import route into the defect: an empty set adopts the file's identity while another
        # loaded set already answers to it.
        'name': "import: adopt an identity another loaded set holds",
        'file': "src/mcp.rs",
        'find': "            && store.set_id().ok().flatten() != Some(offered)\n            && let Some(holder)",
        'replace': "            && false\n            && let Some(holder)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The store's own refusal is what `drop` will lean on to skip a read-only set's safety copy,
        # so each session entry point needs proof that it refuses on its own.
        'name': "read-only: insert takes a session write",
        'file': "src/store.rs",
        'find': "    pub fn insert(&mut self, memory: &Memory) -> Result<(), StoreError> {\n        self.refuse_session_writes_if_read_only()?;",
        'replace': "    pub fn insert(&mut self, memory: &Memory) -> Result<(), StoreError> {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "read-only: carry_forward takes a session write",
        'file': "src/store.rs",
        'find': "    pub fn carry_forward(&mut self, memory: &Memory) -> Result<(), StoreError> {\n        self.refuse_session_writes_if_read_only()?;",
        'replace': "    pub fn carry_forward(&mut self, memory: &Memory) -> Result<(), StoreError> {",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "read-only: forget takes a session write",
        'file': "src/store.rs",
        'find': "    ) -> Result<(), StoreError> {\n        self.refuse_session_writes_if_read_only()?;\n        // Read before the row goes",
        'replace': "    ) -> Result<(), StoreError> {\n        // Read before the row goes",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # A mode chosen after records arrived would let a set this machine wrote to be dropped
        # without a safety copy.
        'name': "read-only: a set holding records can be made read-only",
        'file': "src/store.rs",
        'find': "        if self.holds_any_record()? {\n            return Err(StoreError::ModeIsChosenAtCreation);",
        'replace': "        if false {\n            return Err(StoreError::ModeIsChosenAtCreation);",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "read-only: the policy lets a read-only set be written",
        'file': "src/store.rs",
        'find': "            Self::ReadWrite => true,\n            Self::ReadOnly | Self::Evaluation => false,",
        'replace': "            Self::ReadWrite => true,\n            Self::ReadOnly | Self::Evaluation => true,",
        'tests': ["store", "sets"],
        'expect': "caught",
    },
    {
        # The stored spelling has to read back as the mode that wrote it, or a restart quietly makes
        # every read-only set writable -- or refuses to open it.
        'name': "read-only: the stored mode does not read back",
        'file': "src/store.rs",
        'find': "            Some(spelled) if spelled == LoadMode::ReadOnly.as_str() => Ok(LoadMode::ReadOnly),",
        'replace': "            Some(spelled) if spelled == \"readonly\" => Ok(LoadMode::ReadOnly),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "read-only: create ignores the mode it was given",
        'file': "src/mcp.rs",
        'find': "        if mode == LoadMode::ReadOnly {\n            self.stores",
        'replace': "        if false {\n            self.stores",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Without it, a set imported with its publishers' identity records answers a write with
        # *which of these is you* instead of saying it is read-only.
        'name': "read-only: author_for asks who you are before refusing",
        'file': "src/mcp.rs",
        'find': "        // is you* -- which is meaningless for a set this machine never writes to, and the store's own\n        // refusal is never reached.\n        self.refuse_if_read_only(scope)?;",
        'replace': "        // is you* -- which is meaningless for a set this machine never writes to, and the store's own\n        // refusal is never reached.",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Absorb merges records before it carries anything forward, so only the up-front refusal
        # stops it; the store's refusal would come partway, if at all.
        'name': "read-only: absorb starts on a read-only set",
        'file': "src/mcp.rs",
        'find': "                let request: AbsorbSet = parse(arguments)?;\n                self.refuse_if_read_only(&request.scope)?;",
        'replace': "                let request: AbsorbSet = parse(arguments)?;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `identify` writes only machine state, which the store does not guard, so this refusal is
        # the only one it meets.
        'name': "read-only: identify points a read-only set at an author",
        'file': "src/mcp.rs",
        'find': "                let request: IdentifyAuthor = parse(arguments)?;\n                self.refuse_if_read_only(&request.scope)?;",
        'replace': "                let request: IdentifyAuthor = parse(arguments)?;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `mint` writes an identity record through `insert`, whose own refusal names the set as
        # read-only too, so the test cannot tell which layer refused. The up-front check is kept so
        # the refusal comes before any other work, not because anything here proves it matters.
        'name': "read-only: mint reaches the store before refusing",
        'file': "src/mcp.rs",
        'find': "                let request: MintAuthor = parse(arguments)?;\n                self.refuse_if_read_only(&request.scope)?;",
        'replace': "                let request: MintAuthor = parse(arguments)?;",
        'tests': ["sets"],
        'expect': "survives",
    },
    {
        # The prose is the only place a flattening consumer learns an action's optional arguments.
        'name': "action prose: leave out optional arguments",
        'file': "src/mcp.rs",
        'find': "        if self.optional.is_empty() {\n            return taken;",
        'replace': "        if true {\n            return taken;",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The unconditional save is the rule this exception is carved out of: a set this machine
        # could have written to must be written out before its database goes.
        'name': "drop: no safety copy owed for a read-write set",
        'file': "src/mcp.rs",
        'find': "        let destination = match (save_to, mode.owes_a_safety_copy()) {",
        'replace': "        let destination = match (save_to, false) {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # And the other direction: a read-only set that still demanded `save_to` would lose the
        # reason to hold a set read-only at all.
        'name': "drop: a safety copy owed for every set",
        'file': "src/store.rs",
        'find': "    pub const fn owes_a_safety_copy(self) -> bool {\n        self.accepts_session_writes()",
        'replace': "    pub const fn owes_a_safety_copy(self) -> bool {\n        true",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `save_to` given anyway is honoured rather than dropped on the floor: a path that silently
        # does nothing is the control `decision-placement` warns against.
        'name': "drop: ignore a save_to given for a read-only set",
        'file': "src/mcp.rs",
        'find': "            (Some(path), _) => Some((Destination::File(path.to_path_buf()), path)),\n            (None, false) => None,",
        'replace': "            (Some(path), true) => Some((Destination::File(path.to_path_buf()), path)),\n            (Some(_), false) | (None, false) => None,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A drop that saved nothing and said nothing reads exactly like one that saved something.
        'name': "drop: say nothing about the skipped safety copy",
        'file': "src/mcp.rs",
        'find': "        if destination.is_none() {\n            result[\"safety_copy_skipped\"]",
        'replace': "        if false {\n            result[\"safety_copy_skipped\"]",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The machine's own copy is what wins; without this a read-only copy beside it withholds both
        # and locks someone out of their own set at a restart.
        'name': "load-once: no copy wins when one of them is read-write",
        'file': "src/mcp.rs",
        'find': "            (_, [one]) => Some(one.clone()),",
        'replace': "            (_, [_one]) => None,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Two read-write copies have no winner: taking the first would resolve identities by name
        # order, which is the defect `two-sets-can-share-an-identity` was about.
        'name': "load-once: the first read-write copy wins even when there are two",
        'file': "src/mcp.rs",
        'find': "            (_, [one]) => Some(one.clone()),\n            _ => None,",
        'replace': "            (_, [one, ..]) => Some(one.clone()),\n            _ => None,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A lone copy loads whatever its mode -- the ordinary case for every set on the machine.
        'name': "load-once: a lone copy does not load",
        'file': "src/mcp.rs",
        'find': "            ([(only, _)], _) => Some(only.clone()),",
        'replace': "            ([(_only, _)], _) => None,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Dropping a copy can leave a lone copy, or leave one read-write copy among several, and
        # neither loads until the identity is settled again.
        'name': "load-once: drop leaves the survivors withheld",
        'file': "src/mcp.rs",
        'find': "        if let Some(identity) = identity {\n            self.settle_identity(identity);",
        'replace': "        if false && let Some(identity) = identity {\n            self.settle_identity(identity);",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A read-only set that pulled the collecting and investigating marks would be asked for
        # findings and evidence it cannot store.
        'name': "watch marks: a read-only set pulls every watch mark",
        'file': "src/store.rs",
        'find': "            Self::ReadOnly => &[SIGNPOST_MARK],",
        'replace': "            Self::ReadOnly => &[SIGNPOST_MARK, COLLECT_MARK, INVESTIGATION_MARK],",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # And the other way: a read-write set whose signposts nobody recalls steers nothing.
        'name': "watch marks: a read-write set pulls none",
        'file': "src/store.rs",
        'find': "            Self::ReadWrite => &[SIGNPOST_MARK, COLLECT_MARK, INVESTIGATION_MARK],",
        'replace': "            Self::ReadWrite => &[],",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The convention sends a session to read this field; without it there is nothing to read and
        # no watch list is ever pulled.
        'name': "watch marks: status does not name them",
        'file': "src/mcp.rs",
        'find': "                \"watch_marks\": mode.watch_marks(),",
        'replace': "                \"watch_marks\": Vec::<String>::new(),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Counted since the last calibration, like `calibrate` counts it. Counting every labelled
        # query would report a set as ready for ever, on evidence already consumed.
        'name': "calibration report: count every label, not those since the last calibration",
        'file': "src/store.rs",
        'find': "            \"SELECT count(*) FROM query_log WHERE (used = 1 OR scoped = 1) AND id > ?1\",",
        'replace': "            \"SELECT count(*) FROM query_log WHERE (used = 1 OR scoped = 1) AND ?1 >= 0\",",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # The floor cannot be asked for any other way: `calibrate` sets one and `uncalibrate` removes
        # one, so both other routes change the answer.
        'name': "calibration report: status does not report the floor",
        'file': "src/mcp.rs",
        'find': "                \"non_match_floor\": store\n                    .floor()\n                    .map_err(|error| format!(\"could not read the set's floor: {error}\"))?,",
        'replace': "                \"non_match_floor\": Value::Null,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "calibration report: status does not say how much evidence has accrued",
        'file': "src/mcp.rs",
        'find': "                \"labelled_since_calibration\": labelled,",
        'replace': "                \"labelled_since_calibration\": 0,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A session that is never told the evidence arrived will not calibrate: `scatterhoard` sat
        # past the threshold for weeks with nothing saying so.
        'name': "calibration report: never offer calibration",
        'file': "src/mcp.rs",
        'find': "            if labelled >= LABELS_PER_CALIBRATION {",
        'replace': "            if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The name is the mark: a real set able to carry `eval-` would make it a convention rather
        # than a fact, and a session could not read a name as *these are proposals*.
        'name': "evaluation naming: a review copy's name does not read as one",
        'file': "src/memory/scope.rs",
        'find': "        self.0.len() >= EVALUATION_PREFIX.len()\n            && self.0[..EVALUATION_PREFIX.len()].eq_ignore_ascii_case(EVALUATION_PREFIX)",
        'replace': "        false",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # The label says which copy this is. A name without one defeats having several reviews open.
        'name': "evaluation naming: accept a label that says nothing",
        'file': "src/memory/scope.rs",
        'find': "        if label.is_empty() {\n            return Err(ValidationError::Empty {\n                field: \"an evaluation label\",",
        'replace': "        if false {\n            return Err(ValidationError::Empty {\n                field: \"an evaluation label\",",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # A set nobody adopted must not ask a session for findings or evidence.
        'name': "evaluation mode: pulls watch marks",
        'file': "src/store.rs",
        'find': "            Self::Evaluation => &[],",
        'replace': "            Self::Evaluation => &[SIGNPOST_MARK],",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': "evaluation mode: takes a session's writes",
        'file': "src/store.rs",
        'find': "            Self::ReadOnly | Self::Evaluation => false,",
        'replace': "            Self::ReadOnly => false,\n            Self::Evaluation => true,",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # `create` makes on-disk sets under ordinary names; an evaluation load is neither.
        'name': "evaluation mode: create makes one",
        'file': "src/mcp.rs",
        'find': "            CreatableMode::ReadOnly => Self::ReadOnly,",
        'replace': "            CreatableMode::ReadOnly => Self::Evaluation,",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A review copy carries its source's identity; counting it as a holder would withhold the
        # reviewer's own set the moment they opened a review of it.
        'name': "evaluation load: a review copy holds its source's identity",
        'file': "src/mcp.rs",
        'find': "            // review of it. `documents/design/load-modes.md`.\n            .filter(|(_, store)| !store.load_mode().is_ok_and(LoadMode::is_evaluation))",
        'replace': "            // review of it. `documents/design/load-modes.md`.\n            .filter(|(_, store)| store.load_mode().is_ok())",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Parked below, only `settle_identity` could release it -- and it ignores review copies.
        'name': "evaluation load: a review copy goes through identity settling",
        'file': "src/mcp.rs",
        'find': "        if store.load_mode().is_ok_and(LoadMode::is_evaluation) {\n            self.stores.insert(scope, store);\n            return;\n        }",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The label is what tells two open reviews apart.
        'name': "evaluation load: a label already loaded is taken again",
        'file': "src/mcp.rs",
        'find': "        if self.stores.contains_key(&scope) || self.withheld.contains_key(&scope) {\n            return Err(format!(\n                \"a copy of {declared} is already loaded for review",
        'replace': "        if false {\n            return Err(format!(\n                \"a copy of {declared} is already loaded for review",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Without the mode the copy is an ordinary set: writable, searched unscoped, and a holder.
        'name': "evaluation load: the copy is loaded as an ordinary set",
        'file': "src/mcp.rs",
        'find': "        store\n            .choose_evaluation()\n            .map_err(|error| format!(\"could not hold the copy for review: {error}\"))?;",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The three boundaries that keep the prefix meaning what it says.
        'name': "evaluation naming: create takes a review copy's name",
        'file': "src/mcp.rs",
        'find': "        if set.is_evaluation() {",
        'replace': "        if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "evaluation naming: a file may declare a review copy",
        'file': "src/memory_file.rs",
        'find': "    if let Scope::Shared(set) = declared\n        && set.is_evaluation()",
        'replace': "    if let Scope::Shared(set) = declared\n        && false",
        'tests': ["memory_file"],
        'expect': "caught",
    },
    {
        'name': "evaluation naming: a stored link may name a review copy",
        'file': "src/store.rs",
        'find': "        self.refuse_links_into_an_evaluation_set(memory)?;",
        'replace': "",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # A review of a set must not stop that set importing its own file: the copy carries the
        # identity and holds it for nothing.
        'name': "evaluation load: a review copy blocks its own set's import",
        'file': "src/mcp.rs",
        'find': "            // Review copies carry the identity and hold it for nothing, so an open review of a set\n            // must not stop that set importing its own file.\n            .filter(|(_, store)| !store.load_mode().is_ok_and(LoadMode::is_evaluation))",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Excluded from the search, not filtered from the result: scores are computed against every
        # store a recall searched, so a copy nobody adopted would move everyone else's ranking.
        'name': "evaluation recall: an unscoped recall searches review copies",
        'file': "src/mcp.rs",
        'find': "            None => self\n                .stores\n                .values_mut()\n                .filter(|store| !store.load_mode().is_ok_and(LoadMode::is_evaluation))\n                .collect(),",
        'replace': "            None => self.stores.values_mut().collect(),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A proposal that reads as the project's record is the failure this mark exists for.
        'name': "evaluation recall: returned memories carry no mark",
        'file': "src/mcp.rs",
        'find': "            .filter(|(_, store)| store.load_mode().is_ok_and(LoadMode::is_evaluation))\n            .map(|(scope, _)| scope.clone())",
        'replace': "            .filter(|(_, store)| store.load_mode().is_err())\n            .map(|(scope, _)| scope.clone())",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A summary index that cannot tell a proposal from the record is an index of claims the
        # project never made.
        'name': "evaluation recall: a summary drops the mark",
        'file': "src/mcp.rs",
        'find': "                if self.evaluation {\n                    summary[\"evaluation\"] = json!(true);",
        'replace': "                if false {\n                    summary[\"evaluation\"] = json!(true);",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The mark without its meaning is a field a reader has to guess at.
        'name': "evaluation recall: never explain the mark",
        'file': "src/mcp.rs",
        'find': "        result[\"under_review_note\"] = json!(UNDER_REVIEW_NOTE);",
        'replace': "        let _ = UNDER_REVIEW_NOTE;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A contributor's link attached to whichever copy this machine happens to hold reads as
        # agreement the contributor never expressed.
        'name': "surfaced links: match the link's set by name rather than identity",
        'file': "src/mcp.rs",
        'find': "                Some(wanted) => held.identity == Some(*wanted),",
        'replace': "                Some(_) => held.scope.exports_to() == Some(set),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Without the mode the list is three equivalent names, and the reviewer cannot tell their own
        # copy from the copies under review.
        'name': "surfaced links: do not say how each candidate is held",
        'file': "src/mcp.rs",
        'find': "            .map(|held| json!({\"scope\": WireScope(&held.scope), \"mode\": held.mode}))",
        'replace': "            .map(|held| json!({\"scope\": WireScope(&held.scope), \"mode\": \"read-write\"}))",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Only for a memory under review: an ordinary set resolves its links, so candidates there are
        # noise.
        'name': "surfaced links: surface them for every set",
        'file': "src/mcp.rs",
        'find': "    if !reviewing || view.get(\"links\").is_none() {",
        'replace': "    if view.get(\"links\").is_none() {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Every question a reviewer asks while reading someone else's branch would otherwise set that
        # copy's floor, from a corpus nobody keeps.
        'name': "evaluation recall: a review copy logs its queries",
        'file': "src/store.rs",
        'find': "            if store.load_mode().is_ok_and(LoadMode::is_evaluation) {\n                continue;\n            }",
        'replace': "",
        'tests': ["floor"],
        'expect': "caught",
    },
    {
        # Two servers share a home, so a label taken by another server's review is taken here too:
        # opening its database would put two servers on one file.
        'name': "evaluation load: take a label another server's review already holds",
        'file': "src/mcp.rs",
        'find': "        if path.exists() {\n            return Err(format!(\n                \"a copy of {declared} is already held for review",
        'replace': "        if false {\n            return Err(format!(\n                \"a copy of {declared} is already held for review",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The 512 in `encode`'s default is not a limit the model imposes -- a static model
        # mean-pools and has no context window -- and what it removed was the end of the embedding
        # input, which is a memory's `why` and then its keywords. Restoring the default is silent:
        # every memory still embeds, and only the tail of the long ones stops reaching the vector.
        'name': "embedding: go back to truncating the input at 512 tokens",
        'file': "src/embedder.rs",
        'find': ".encode_with_args(texts, None, BATCH)",
        'replace': ".encode_with_args(texts, Some(512), BATCH)",
        'tests': ["embedder"],
        'expect': "caught",
    },
    {
        # The encoding is in the identity so that re-encoded vectors and truncated ones cannot sit
        # in one store comparing themselves to each other. Dropping it leaves the weights hash
        # agreeing, which is what makes the loss invisible rather than merely wrong.
        'name': "model identity: record the weights and not how text is encoded",
        'file': "src/embedder.rs",
        'find': 'ModelIdentity(format!("{}+{ENCODING}", model.as_str()))',
        'replace': "ModelIdentity(format!(\"{}\", model.as_str()))",
        'tests': ["model_identity"],
        'expect': "caught",
    },
    {
        # Raising the limit out of reach is how this guard would fail in practice -- not by being
        # deleted, but by a number nobody re-derived. Both write doors are covered by one mutation
        # because both read this constant.
        'name': "memory length: raise the limit past anything a caller would write",
        'file': "src/embedding.rs",
        'find': "pub const MAX_EMBEDDING_INPUT_CHARS: usize = 2000;",
        'replace': "pub const MAX_EMBEDDING_INPUT_CHARS: usize = 2_000_000;",
        'tests': ["mcp", "correct"],
        'expect': "caught",
    },
    # -----------------------------------------------------------------------------------------
    # `several-memory-files-per-directory`, 2026-09-26: the `directory` form finds a set's file by
    # the identity in each header, then by declared name, and names a new file after its set.
    # -----------------------------------------------------------------------------------------
    {
        # Identity is what makes a file's name free. Matching on the declared name alone still finds
        # an unrenamed file, so only a test that renames one notices.
        'name': "directory form: find a set's file by declared name only, never by identity",
        'file': "src/mcp.rs",
        'find': "        .filter(|(_, header)| identity.is_some() && header.set_id == identity)",
        'replace': "        .filter(|(_, header)| false && header.set_id == identity)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The name fallback is how a set with no identity yet -- a first import -- finds its file.
        'name': "directory form: drop the fallback to the declared name",
        'file': "src/mcp.rs",
        'find': "            .filter(|(_, header)| header.scope == *scope)",
        'replace': "            .filter(|(_, header)| false && header.scope == *scope)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Two files answering to one set: taking the first is a guess about which holds the work.
        'name': "directory form: take the first of several candidate files",
        'file': "src/mcp.rs",
        'find': "        [only] => Ok(Some((*only).clone())),",
        'replace': "        [only, ..] => Ok(Some((*only).clone())),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A damaged file may be the set's own; skipping it writes a second file beside it.
        'name': "directory form: skip a memory file whose header cannot be read",
        'file': "src/mcp.rs",
        'find': "            })?;\n        found.push((path, header));",
        'replace': "            });\n        let Ok(header) = header else { continue };\n        found.push((path, header));",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # macOS's `._name` files are not memory files whatever their extension.
        'name': "directory form: read hidden files as memory files",
        'file': "src/mcp.rs",
        'find': "            .is_some_and(|name| name.starts_with('.'));",
        'replace': "            .is_some_and(|name| name.starts_with('.') && false);",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `con.json` is a checkout that fails on Windows wherever it was written.
        'name': "file naming: let a Windows device name through",
        'file': "src/mcp.rs",
        'find': "    let stem = if WINDOWS_RESERVED_NAMES.contains(&first.to_ascii_lowercase().as_str()) {",
        'replace': "    let stem = if WINDOWS_RESERVED_NAMES.contains(&first) && false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `Notes.json` and `notes.json` are one file on a Mac.
        'name': "file naming: compare taken names with case",
        'file': "src/mcp.rs",
        'find': "    while taken.contains(&candidate.to_lowercase()) {",
        'replace': "    while taken.contains(&candidate) {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `absorb` and `evaluate` in `directory` form search the headers for the set the request
        # names. Searching for nothing in particular -- whatever shared set is first -- is the guess
        # the named set exists to prevent.
        'name': "named file: search a directory without a set named",
        'file': "src/mcp.rs",
        'find': "                let Some(set) = set else {\n                    return Err(format!(",
        'replace': "                let Some(set) = set else {\n                    if let Some((first, _)) = memory_files_in(&directory.join(RESERVED_DIRECTORY))?.into_iter().next() {\n                        return Ok(first);\n                    }\n                    return Err(format!(",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # With `file`, a named set is checked against the header rather than used to find anything.
        'name': "named file: accept any shared set whatever was named",
        'file': "src/mcp.rs",
        'find': "    set.map_or(memory_file::Expectation::AnyShared, |set| {",
        'replace': "    set.filter(|_| false).map_or(memory_file::Expectation::AnyShared, |set| {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `the-reserved-directory-accepts-the-local-set`: the personal set in a checkout's reserved
        # directory is published by the next commit, or written into by a crafted one.
        'name': "reserved directory: let the local set through",
        'file': "src/mcp.rs",
        'find': "    if *scope != Scope::Local {\n        return Ok(());\n    }",
        'replace': "    if true {\n        return Ok(());\n    }",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The rule itself: a shared set's first memory has to be its charter (`set-settled-at-first-write`).
        'name': 'charter first: refuse no first write',
        'file': 'src/mcp.rs',
        'find': '        Ok(empty.then(|| {',
        'replace': '        Ok((false && empty).then(|| {',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # The way through the refusal is the charter itself; refusing it too would leave a new set unwritable.
        'name': 'charter first: refuse the charter too',
        'file': 'src/mcp.rs',
        'find': '        if is_charter {\n            return Ok(None);\n        }',
        'replace': '        if is_charter && false {\n            return Ok(None);\n        }',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # `local` is never created and never published, and has no charter to require at its first write.
        'name': "charter first: refuse local's first write too",
        'file': 'src/mcp.rs',
        'find': '        let Scope::Shared(_) = scope else {\n            return Ok(None);\n        };\n        let is_charter',
        'replace': '        let is_charter',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # Forgetting is a write, so a set holding only tombstones has been written to and is not refused.
        'name': 'charter first: count a set that has only forgotten as never written',
        'file': 'src/store.rs',
        'find': '        Ok(self.count()? == 0 && self.forgotten_count()? == 0)',
        'replace': '        Ok(self.count()? == 0)',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # Only the first write: a set already holding memories without a charter came from someone else.
        'name': 'charter first: refuse a set that already holds memories',
        'file': 'src/store.rs',
        'find': '        Ok(self.count()? == 0 && self.forgotten_count()? == 0)',
        'replace': '        Ok(self.forgotten_count()? == 0)',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # Resolving the author mints a record, which is the set's first write and gives it an identity, so
        # a refusal asked afterwards leaves a set it refused no longer empty.
        'name': 'charter first: ask after minting the author',
        'file': 'src/mcp.rs',
        'find': '        if let Some(refusal) =\n            self.charter_first_refusal(&params.scope, params.keywords.as_deref())?\n        {\n            return Err(refusal);\n        }\n\n        // Nothing can refuse from here on, so minting is safe.\n        let author = self.author_for(&params.scope)?;',
        'replace': '        // Nothing can refuse from here on, so minting is safe.\n        let author = self.author_for(&params.scope)?;\n        if let Some(refusal) =\n            self.charter_first_refusal(&params.scope, params.keywords.as_deref())?\n        {\n            return Err(refusal);\n        }',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A read-only set takes no writes at all; telling an empty one to write a charter names a remedy it
        # cannot use.
        'name': 'charter first: advise a read-only set to write a charter',
        'file': 'src/mcp.rs',
        'find': '        self.refuse_if_read_only(&params.scope)?;\n        if let Some(refusal) =',
        'replace': '        if let Some(refusal) =',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # `route-where` promises that `hoard_sets` says how to begin a set at the moment it is made: name
        # who writes, then the charter.
        'name': 'create: say nothing about the charter',
        'file': 'src/mcp.rs',
        'find': '            "note": note,',
        'replace': '            "unsaid": note,',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # The account name is `local`'s default and nothing else's: a shared set's name is how you are known
        # where it is published, which varies by set.
        'name': 'author: give a shared set the account name',
        'file': 'src/mcp.rs',
        'find': '        let Scope::Local = scope else {\n            return Ok(None);\n        };\n        let recorded',
        'replace': '        let recorded',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # `local` takes the account name so that a new user's first personal memory needs no extra step.
        'name': 'author: give local no account-name default',
        'file': 'src/mcp.rs',
        'find': '        Ok(recorded.or_else(|| self.account_name.clone()))',
        'replace': '        Ok(recorded)',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # Where nothing names an author the write is refused, never recorded under a name nobody chose --
        # which is what `unknown` was on Windows.
        'name': 'author: write under a made-up name when nobody is named',
        'file': 'src/mcp.rs',
        'find': '                None => return Err(nobody_named_refusal(scope)),',
        'replace': '                None => Some(Author::new("unknown").expect("valid")),',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A set holding identity records this machine has not chosen between asks which is you; minting
        # another would fork one person into two identities.
        'name': 'author: mint beside identity records instead of asking',
        'file': 'src/mcp.rs',
        'find': '        let name = if holds_records {',
        'replace': '        let name = if false {',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # `mint` into a new set is that set's first write and gives it its identity; the durable map row keyed
        # on it is what lets a re-added copy of the set recognise you.
        'name': 'mint: stop remembering which record is yours',
        'file': 'src/mcp.rs',
        'find': '        if let Some(set_id) = set_id {\n            self.remember_author(scope, set_id, record)?;\n        }\n\n        Ok(json!({\n            "scope": WireScope(scope),\n            "minted"',
        'replace': '        let _ = (set_id, record);\n\n        Ok(json!({\n            "scope": WireScope(scope),\n            "minted"',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # `status` has to say how an unclaimed shared set is claimed, not promise a claim the write refuses.
        'name': "author note: promise a claim a shared set's write will refuse",
        'file': 'src/mcp.rs',
        'find': '        None => format!(\n            "{state} The next write here is refused until you say who you are in it',
        'replace': '        None => format!(\n            "{state} The next write claims it; until you say who you are in it',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # `unknown-labels-carried-opaquely`: nothing walks an edge it does not understand. The
        # negated form this replaced made `Unrecognised` walkable the day it was added, and no
        # compiler error said so.
        'name': 'unknown labels: walk an unrecognised label',
        'file': 'src/traversal.rs',
        'find': '        | LinkLabel::InstanceOf => true,\n        LinkLabel::Supersedes | LinkLabel::SupersededBy | LinkLabel::Unrecognised(_) => false,',
        'replace': '        | LinkLabel::InstanceOf\n        | LinkLabel::Unrecognised(_) => true,\n        LinkLabel::Supersedes | LinkLabel::SupersededBy => false,',
        'tests': ['traversal'],
        'expect': "caught",
    },
    {
        # A known spelling must never parse as `Unrecognised`, or a file could carry `supersedes`
        # opaquely past every check that matches on it.
        'name': 'unknown labels: parse every name as unrecognised',
        'file': 'src/memory/link.rs',
        'find': '            .find(|label| label.name() == name)',
        'replace': '            .find(|_| false)',
        'tests': ['memory_model'],
        'expect': "caught",
    },
    {
        # Written back exactly as read, so a newer build's checksums still describe the bytes.
        'name': 'unknown labels: export an unrecognised label as a known one',
        'file': 'src/memory/link.rs',
        'find': '            Self::Unrecognised(label) => label.as_str(),',
        'replace': '            Self::Unrecognised(_) => "related_to",',
        'tests': ['memory_model', 'unknown_labels'],
        'expect': "caught",
    },
    {
        # Authoring stays strict: with nothing to restate, an unknown label is a typo.
        'name': 'unknown labels: accept an authored unknown label',
        'file': 'src/mcp.rs',
        'find': '        .filter(|link| !link.label.is_recognised() && !restates(link))',
        'replace': '        .filter(|link| false && !restates(link))',
        'tests': ['unknown_labels'],
        'expect': "caught",
    },
    {
        # The pair is restated, not the label: the same label aimed somewhere new is a new claim.
        'name': 'unknown labels: restate a label without its target',
        'file': 'src/mcp.rs',
        'find': '            .any(|old| old.label == link.label && old.target.id() == link.target.id())',
        'replace': '            .any(|old| old.label == link.label)',
        'tests': ['unknown_labels'],
        'expect': "caught",
    },
    {
        # A supersession may restate from the memory it supersedes; without that, no unknown label
        # survives a supersession.
        'name': 'unknown labels: memorize restates from nothing',
        'file': 'src/mcp.rs',
        'find': '                Some(store) => superseded_links(store, &links, &params.scope, holder)?,',
        'replace': '                Some(_) => Vec::new(),',
        'tests': ['unknown_labels'],
        'expect': "caught",
    },
    {
        # Amend replaces the whole link array, the same trap as supersession, so it restates from
        # the memory being amended.
        'name': 'unknown labels: amend restates from nothing',
        'file': 'src/mcp.rs',
        'find': '            && let Some(refusal) = unknown_label_refusal(links, &memory.links)',
        'replace': '            && let Some(refusal) = unknown_label_refusal(links, &[])',
        'tests': ['unknown_labels'],
        'expect': "caught",
    },
    {
        # And amend refuses on the same terms as memorize: without it, amend authors unknown labels.
        'name': 'unknown labels: amend skips the refusal',
        'file': 'src/mcp.rs',
        'find': '            && let Some(refusal) = unknown_label_refusal(links, &memory.links)',
        'replace': '            && let Some(refusal) = None::<String>.filter(|_| !links.is_empty() && memory.links.is_empty())',
        'tests': ['unknown_labels'],
        'expect': "caught",
    },
    {
        # Decided 2026-09-28: an `enum` has a validating client refuse the restatement before the
        # server sees it, which makes the restating rule a guard no client can reach.
        'name': 'unknown labels: publish the vocabulary as an enum again',
        'file': 'src/mcp.rs',
        'find': '                "label": {\n                    "type": "string",\n                    "description": format!(',
        'replace': '                "label": {\n                    "type": "string",\n                    "enum": LinkLabel::ALL.iter().map(LinkLabel::name).collect::<Vec<_>>(),\n                    "description": format!(',
        'tests': ['unknown_labels', 'mcp'],
        'expect': "caught",
    },
    {
        # Claude Desktop drops property descriptions, so the tool's own description is where a
        # session there sees the vocabulary at all. Decided 2026-09-28 with the human partner.
        'name': 'unknown labels: leave the vocabulary out of the tool descriptions',
        'file': 'src/mcp.rs',
        'find': '        "A link\'s `label` is one of {}; one copied from the memory being superseded or amended \\\n         keeps its label as written.",\n        label_vocabulary()',
        'replace': '        "A link\'s `label` names what it claims{}.",\n        String::new()',
        'tests': ['unknown_labels'],
        'expect': "caught",
    },
    {
        # `a-link-can-reach-the-local-set-by-its-identity`: an identity chooses only among the stores
        # a name could reach. Without the guard a crafted file's `superseded_by` carrying `local`'s
        # identity substitutes a personal memory, and a shared edge walks into one.
        'name': 'holder_of: let an identity reach the local set',
        'file': 'src/store.rs',
        'find': '            if store.scope.exports_to().is_some() && store.set_id()? == Some(wanted) {',
        'replace': '            if store.set_id()? == Some(wanted) {',
        'tests': ['cross_store'],
        'expect': "caught",
    },
    {
        # `absorb-internalises-links-to-memories-that-did-not-arrive`: a link is made internal only
        # if its target arrived in the file. Without the second half, a partial file exchanges a
        # working cross-set link for a same-set one to an id this set lacks.
        'name': 'absorb: internalise a link whose target did not arrive',
        'file': 'src/mcp.rs',
        'find': '            |link: &Link| names_absorbed(link) && will_hold.contains(&link.target.id());',
        'replace': '            |link: &Link| names_absorbed(link);',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A memory with nothing this file lets it internalise is left alone: its replacement would
        # differ from it only in id.
        'name': 'absorb: supersede a memory with nothing to internalise',
        'file': 'src/mcp.rs',
        'find': '                    if memory.links.iter().any(internalises) {',
        'replace': '                    if true {',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A partial file is otherwise indistinguishable from a whole one, so the links it left
        # cross-set are named in the result.
        'name': 'absorb: stop reporting links left cross-set',
        'file': 'src/mcp.rs',
        'find': '        if !not_internalised.is_empty() {',
        'replace': '        if false {',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A move done in steps: a target taken in by an earlier absorb is the memory the link means.
        # Decided with the human partner 2026-09-28.
        'name': 'absorb: count only this file, not what earlier absorbs brought',
        'file': 'src/mcp.rs',
        'find': '            .chain(held_memories.iter().copied())\n',
        'replace': '',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A tombstone keeps the id of a memory this set forgot; a link made internal to it dangles.
        'name': 'absorb: count a forgotten memory as held',
        'file': 'src/mcp.rs',
        'find': '                if matches!(record, memory_file::Record::Memory(_)) {\n                    held_memories.insert(record.id());',
        'replace': '                if true {\n                    held_memories.insert(record.id());',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A forget is one-way at import (`merge-without-a-recorded-ancestor`, 2026-09-28): a memory
        # never replaces a tombstone the store holds, whatever the base says.
        'name': 'reconcile: let a memory replace a tombstone',
        'file': 'src/reconcile.rs',
        'find': '    if stored_is_tombstone {\n        return Outcome::KeptForgotten;\n    }',
        'replace': '    let _ = stored_is_tombstone;',
        'tests': ['reconcile', 'sets'],
        'expect': "caught",
    },
    {
        # The import's half: telling the decision which held ids are tombstones.
        'name': 'import: never say a held id is a tombstone',
        'file': 'src/mcp.rs',
        'find': '                        forgotten_here.contains(&record.id()),',
        'replace': '                        false,',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # A reverted file lands here too, so the caller is told it did not happen and how to undo
        # a forget on purpose.
        'name': 'import: stop naming the memories kept forgotten',
        'file': 'src/mcp.rs',
        'find': '        if !kept_forgotten.is_empty() {',
        'replace': '        if false {',
        'tests': ['sets'],
        'expect': "caught",
    },
    {
        # `concurrent-first-writes-can-mint-two-identities`: the claim returns what the set holds
        # after the write, not what this call generated -- the later of two first writes loses.
        'name': 'set identity: a claim trusts the identity it generated',
        'file': 'src/store.rs',
        'find': '        self.fill_set_id(id)?;\n        self.set_id()?\n            .ok_or_else(|| StoreError::Database(rusqlite::Error::QueryReturnedNoRows))\n',
        'replace': '        self.fill_set_id(id)?;\n        Ok(id)\n',
        'tests': ['concurrency'],
        'expect': "caught",
    },
    {
        # Adoption says whether it took the offer, which is what lets import refuse a file whose
        # identity lost to a mint on another connection.
        'name': 'set identity: adoption claims it took an offer it lost',
        'file': 'src/store.rs',
        'find': '        self.fill_set_id(offered)\n',
        'replace': '        self.fill_set_id(offered).map(|_| true)\n',
        'tests': ['concurrency'],
        'expect': "caught",
    },
    {
        # `the-silenced-recall-note-carries-source-indentation`: a line break that loses its `\`
        # serves the next line's indentation. Caught as served and at the source.
        'name': 'served text: indentation back in the silenced recall note',
        'file': 'src/mcp.rs',
        'find': "which is a level calibrated \\\n                 from what",
        'replace': "which is a level calibrated                  from what",
        'tests': ['mcp', 'served_text'],
        'expect': "caught",
    },
    {
        # The class, not one message: a refusal no test provokes is still read at the source.
        'name': 'served text: indentation back in the cross-set authority refusal',
        'file': 'src/mcp.rs',
        'find': "change the standing of a memory in \\\n",
        'replace': "change the standing of a memory in          ",
        'tests': ['served_text'],
        'expect': "caught",
    },
    {
        # `keyword-recall-ignores-already-known`: each store skips held ids before loading them.
        'name': 'keyword cursor: load held ids before discarding them',
        'file': 'src/store.rs',
        'find': '            if skip.contains(&id) {\n                continue;\n            }',
        'replace': '            if false {\n                continue;\n            }',
        'tests': ['mcp', 'other_paths'],
        'expect': "survives",
        'reason': "A cost guard, not a behavioural one: without it every held id is loaded and then "
                  "left out by the merge's own check on the memory returned, so every result is "
                  "identical. What it saves is loading the top of the ranking -- the ids a caller "
                  "has already paged past -- on every page, which no test can observe.",
    },
    {
        # A hit substituted by another set's replacement is only known to be held once the merge
        # makes the substitution, so the merge asks again.
        'name': 'keyword recall: take a substituted hit the caller holds',
        'file': 'src/store.rs',
        'find': '        if already_known.contains(&hit.memory.id) {\n            continue;\n        }',
        'replace': '        if false {\n            continue;\n        }',
        'tests': ['cross_store'],
        'expect': "caught",
    },
    {
        # The tool's half: handing the held ids to the keyword path at all.
        'name': 'keyword recall: the tool drops the held ids',
        'file': 'src/mcp.rs',
        'find': '                    limit,\n                    &already_known,\n',
        'replace': '                    limit,\n                    &[],\n',
        'tests': ['mcp'],
        'expect': "caught",
    },
    {
        # Keyword retrieval reads each store in bounded batches since 2026-09-29; a cursor that
        # stopped after its first batch would silently cap every store at `KEYWORD_BATCH` matches.
        'name': 'keyword cursor: stop after the first batch',
        'file': 'src/store.rs',
        'find': '        self.exhausted = rows.len() < KEYWORD_BATCH;',
        'replace': '        self.exhausted = true;',
        'tests': ['other_paths'],
        'expect': "caught",
    },
    {
        # A batch continues from the last `(matches, id)` read; comparing the id the wrong way
        # re-reads the start of a tier and skips the rest of it.
        'name': 'keyword cursor: continue a tier in the wrong direction',
        'file': 'src/store.rs',
        'find': '"HAVING COUNT(*) < ? OR (COUNT(*) = ? AND memory_id > ?)"',
        'replace': '"HAVING COUNT(*) < ? OR (COUNT(*) = ? AND memory_id < ?)"',
        'tests': ['other_paths'],
        'expect': "caught",
    },
    {
        # A one-word sentence -- a list number, a bare answer -- is not a boundary, or a numbered
        # opening summarises to `1.…`. `a-numbered-opening-summarises-to-its-number`.
        'name': "summary: let a one-word sentence end the line",
        'file': "src/memory/text.rs",
        'find': "            && line[..at]\n                .iter()\n                .rposition(|c| *c == ' ')\n                .is_some_and(|space| !is_terminator(space - 1))",
        'replace': "",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # The second half of the same guard: a one-word sentence after an earlier one, where a
        # space does precede it and only the terminator before that space says it is one word.
        'name': "summary: count only the first word of the line as a one-word sentence",
        'file': "src/memory/text.rs",
        'find': ".is_some_and(|space| !is_terminator(space - 1))",
        'replace': ".is_some()",
        'tests': ["memory_model"],
        'expect': "caught",
    },
    {
        # `identity-counts-test-the-keyword-not-the-record`: what makes a record an identity is
        # that it attributes itself. Asking the keyword instead counts a memory *about* identity
        # records as one, so `status` under-reports and `export` names it among `identities`.
        'name': "count: tell an identity record by its keyword again",
        'file': "src/store.rs",
        'find': "            \"SELECT COUNT(*) FROM memories WHERE created_by <> id\",",
        'replace': "            \"SELECT COUNT(*) FROM memories WHERE id NOT IN (SELECT memory_id FROM keywords WHERE folded = 'hoard-author')\",",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # The complement of the count above, and reported beside it so `records` reconciles.
        'name': "identity_count: tell an identity record by its keyword again",
        'file': "src/store.rs",
        'find': "    pub fn identity_count(&self) -> Result<usize, StoreError> {\n        let total: i64 = self.connection.query_row(\n            \"SELECT COUNT(*) FROM memories WHERE created_by = id\",",
        'replace': "    pub fn identity_count(&self) -> Result<usize, StoreError> {\n        let total: i64 = self.connection.query_row(\n            \"SELECT COUNT(*) FROM memories WHERE id IN (SELECT memory_id FROM keywords WHERE folded = 'hoard-author')\",",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # A set holding only a memory *about* identity records read as already identified, so
        # `author_record_or_mint` would not mint its first identity.
        'name': "holds_any_author_record: tell an identity record by its keyword again",
        'file': "src/store.rs",
        'find': "        let count: i64 = self.connection.query_row(\n            \"SELECT COUNT(*) FROM memories WHERE created_by = id\",",
        'replace': "        let count: i64 = self.connection.query_row(\n            \"SELECT COUNT(*) FROM keywords WHERE folded = 'hoard-author'\",",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # An ordinary memory whose text is a name is not a record holding that name, so it must
        # not make `mint_author` refuse as though minting would fork a person.
        'name': "author_records_named: accept any memory carrying the name",
        'file': "src/store.rs",
        'find': ".prepare(\"SELECT id FROM memories WHERE created_by = id AND content = ?1\")?;",
        'replace': ".prepare(\"SELECT id FROM memories WHERE content = ?1\")?;",
        'tests': ["store"],
        'expect': "caught",
    },
    {
        # Calibrating a floor from a set's own memories asks each claim against the rest; an
        # identity record is a name, not a claim.
        'name': "floor: calibrate from identity records as though they were claims",
        'file': "src/store.rs",
        'find': "            if memory.is_identity_record() {\n                continue;\n            }\n            claims.push(memory);",
        'replace': "            claims.push(memory);",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The budget is named and argued in characters; `str::len()` is bytes, so an em dash costs
        # three. `catalog-budget-counts-bytes-not-characters`.
        'name': "conventions: measure a convention in bytes again",
        'file': "src/conventions/mod.rs",
        'find': "        self.text.chars().count() + self.reinforcement.chars().count()",
        'replace': "        self.text.len() + self.reinforcement.len()",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        # The budget counts the id as well as what is served; leaving it out under-reports the
        # catalog by the length of every id.
        'name': "conventions: leave the id out of what the budget counts",
        'file': "src/conventions/mod.rs",
        'find': "        self.id.chars().count() + self.served_chars()",
        'replace': "        self.served_chars()",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        # `max-chars-is-not-a-bound`. The cursor and the budget are both characters; measuring the
        # document in bytes puts its end past the last character, so a cursor there is served.
        'name': "conventions: measure the catalog document in bytes",
        'file': "src/conventions/mod.rs",
        'find': "    let length = document.chars().count();",
        'replace': "    let length = document.len();",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        # The last line quotes the next cursor, so it is sized for the widest one; sizing it for
        # the narrowest lets a page late in the catalog overrun by the difference in digits.
        'name': "conventions: size the last line for the narrowest cursor",
        'file': "src/conventions/mod.rs",
        'find': "            let overhead = lines(&more_follows(length, budget));",
        'replace': "            let overhead = lines(&more_follows(0, budget));",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        'name': "conventions: leave the aside out of the budget",
        'file': "src/conventions/mod.rs",
        'find': "    let aside_chars = aside.map_or(0, |aside| aside.chars().count() + 1);",
        'replace': "    let aside_chars = 0;",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        # The tested wording: a reader given only the cursor dropped `max_chars` every time, and
        # the next page then fell back to the whole catalog.
        'name': "conventions: tell the reader the cursor but not the budget",
        'file': "src/conventions/mod.rs",
        'find': """r#"The conventions are not loaded yet. Call hoard_conventions again with {{"cursor": {next}, "max_chars": {max_chars}}} for the rest."#""",
        'replace': """r#"The conventions are not loaded yet. Call hoard_conventions again with {{"cursor": {next}}} for the rest. {max_chars}"#""",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        # The least budget that works has to count its own digits in the line it pays for.
        'name': "conventions: name a least budget sized for the refused one's digits",
        'file': "src/conventions/mod.rs",
        'find': "                    .find(|&candidate| candidate > lines(&more_follows(length, candidate)))",
        'replace': "                    .find(|&candidate| candidate > lines(&more_follows(length, budget)))",
        'tests': ["conventions"],
        'expect': "caught",
    },
    {
        # A page is text addressed to its reader; serialising it as a JSON string quotes it and
        # escapes every line break, so the last line is no longer a line.
        'name': "mcp: serve a convention page as a JSON string",
        'file': "src/mcp.rs",
        'find': "            Value::String(text) => json!({\n                \"content\": [{\"type\": \"text\", \"text\": text}],",
        'replace': "            Value::String(text) => json!({\n                \"content\": [{\"type\": \"text\", \"text\": Value::String(text).to_string()}],",
        'tests': ["mcp", "approval"],
        'expect': "caught",
    },
    {
        # A set name is validated the same way wherever it appears, so its own message cannot say
        # which field it is about. `the-set-name-refusal-does-not-say-how-the-other-field-spells-it`.
        'name': "link: refuse a target_set with the bare set-name message",
        'file': "src/memory/link.rs",
        'find': "D::Error::custom(target_set_refusal(&set, &error.to_string()))",
        'replace': "D::Error::custom(error)",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # The scope spelling is the mistake the two fields invite, so it gets the spelling to send
        # rather than only the field's name.
        'name': "link: give a scope-spelled target_set no remedy",
        'file': "src/memory/link.rs",
        'find': "        Some(bare) if SetName::new(bare).is_ok() => format!(",
        'replace': "        Some(bare) if false && SetName::new(bare).is_ok() => format!(",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        # No model is not a second model: the status note names the model to restore and blames no
        # other server. `a-missing-model-reports-as-a-mismatch`.
        'name': "status: describe a missing model as a disagreement between servers again",
        'file': "src/mcp.rs",
        'find': "                 embedded once a model returns. Make `{recorded}` loadable again -- this server \\\n                 reads its model from `SCATTERHOARD_MODEL`, or from `model` in its data \\\n                 directory, and says on startup why loading failed. Keyword search and \\",
        'replace': "                 embedded once a model returns. If another server is running with its own \\\n                 model, fix the disagreement between the two configurations first. Keyword \\\n                 search and \\",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # With no model loaded `reembed` is refused for having nothing to rebuild with, so a recall's
        # reason recommending it names a remedy that cannot be taken.
        'name': "recall: answer a missing model with a rebuild again",
        'file': "src/store.rs",
        'find': "                 in this server, so there is nothing to search them with. Make `{recorded}` \\\n                 loadable again: this server reads its model from `SCATTERHOARD_MODEL`, or from \\\n                 `model` in its data directory. Keyword search, hoard_lookup and traversal are \\",
        'replace': "                 in this server, so there is nothing to search them with. Decide which model \\\n                 should win: `hoard_sets` action \\\"reembed\\\" rebuilds it. Keyword search, \\\n                 hoard_lookup and traversal are \\",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A forgotten replacement is withdrawn here, not held elsewhere; reporting it as absent sends
        # the reader to load a set that will never have it.
        # `a-forgotten-replacement-reads-as-superseded-beyond-this-set`.
        'name': "resolution: report a forgotten replacement as an absent one",
        'file': "src/store.rs",
        'find': "                } else if self.holds_record(current)? {",
        'replace': "                } else if false {",
        'tests': ["correct", "store"],
        'expect': "caught",
    },
    {
        # Only a concession to another set can be re-resolved across stores; an absent id has to stay
        # out of reach, or a memory whose chain ends on one looks up as current.
        'name': "standing: let a chain ending on an absent id read as resolved",
        'file': "src/store.rs",
        'find': "            unresolved: absent,\n            forgotten,\n",
        'replace': "            unresolved: false,\n            forgotten,\n",
        'tests': ["correct", "store", "cross_store", "mcp"],
        'expect': "caught",
    },
    {
        # The same, where a concession to another set is also in play.
        'name': "standing: drop the absent reason when a concession is re-resolved",
        'file': "src/store.rs",
        'find': "    let mut out_of_reach = absent;",
        'replace': "    let mut out_of_reach = false;",
        'tests': ["correct", "store", "cross_store", "mcp"],
        'expect': "caught",
    },
    {
        # Lookup reads standing across stores; dropping the forgotten reason there makes a memory
        # whose only replacement was forgotten look up as current.
        'name': "standing: drop the forgotten reason",
        'file': "src/store.rs",
        'find': "            unresolved: absent,\n            forgotten,\n",
        'replace': "            unresolved: absent,\n            forgotten: false,\n",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # A single store's hit, where the keyword path first resolves it.
        'name': "recall: drop the forgotten reason from a single store's hit",
        'file': "src/store.rs",
        'find': "                None => (id, None, false, unresolved, forgotten),",
        'replace': "                None => (id, None, false, unresolved, false),",
        'tests': ["correct"],
        'expect': "caught",
    },
    {
        # A chain is walked *through* a member this store cannot produce, so a link landing before a
        # forgotten memory reaches what undid the forget. Asked by the human partner 2026-09-30.
        'name': "resolution: stop a chain at a replacement this store cannot produce",
        'file': "src/store.rs",
        'find': "                if seen.insert(replacement) {",
        'replace': "                if seen.insert(replacement) && self.holds_memory(replacement)? {",
        'tests': ["cross_store", "correct", "store"],
        'expect': "caught",
    },
    {
        # The in-links a supersession re-points are named, so the one person holding both texts can
        # say whether each still holds. `supersession-silently-repoints-an-in-link-nobody-reauthored`.
        'name': "supersession: say nothing about same-set in-links it re-points",
        'file': "src/mcp.rs",
        'find': "            if claims(&edge.label) {",
        'replace': "            if false && claims(&edge.label) {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The walk re-points links from other loaded sets too, so they are named with their set.
        'name': "supersession: say nothing about in-links from other sets",
        'file': "src/mcp.rs",
        'find': "            if claims(&reference.label) {",
        'replace': "            if false && claims(&reference.label) {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A supersession pointing at a superseded memory is chain structure the walk is meant to
        # follow, not a claim re-pointed; naming it would fire on every chain extension.
        'name': "supersession: report chain links as re-pointed claims",
        'file': "src/mcp.rs",
        'find': "    let claims =\n        |label: &LinkLabel| !matches!(label, LinkLabel::Supersedes | LinkLabel::SupersededBy);\n    let Some(store)",
        'replace': "    let claims = |_: &LinkLabel| true;\n    let Some(store)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Names differing only in case share one store file on macOS and Windows, and on Linux would
        # collide the moment the data directory moved. `set-names-differing-only-in-case-share-a-store-file`.
        'name': "set name: compare names for a shared file case-sensitively",
        'file': "src/memory/scope.rs",
        'find': "        self.0.eq_ignore_ascii_case(&other.0)",
        'replace': "        self.0 == other.0",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "create: make a set differing only in case from a loaded one",
        'file': "src/mcp.rs",
        'find': "            && let Some(held) = self.case_variant_of(set)\n",
        'replace': "            && let Some(held) = None::<&SetName>\n",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "evaluate: load a review whose label differs only in case from a loaded one",
        'file': "src/mcp.rs",
        'find': "            && let Some(held) = self.case_variant_of(named)\n",
        'replace': "            && let Some(held) = None::<&SetName>\n",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A withheld set still owns its file, so a case variant of it is refused too.
        'name': "case variants: ignore withheld sets",
        'file': "src/mcp.rs",
        'find': "            .chain(self.withheld.keys())\n            .find_map(|scope| match scope {",
        'replace': "            .find_map(|scope| match scope {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A personal store split across two directories is merged back only by absorbing a `local`
        # file into `local`. `a-split-local-store-cannot-be-merged-back`.
        'name': "absorb: refuse a local file into local again",
        'file': "src/mcp.rs",
        'find': "            let expectation = if *scope == Scope::Local {",
        'replace': "            let expectation = if false {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The other direction of the same line: a shared set never takes a `local` file, or personal
        # memories would reach a set that is published.
        'name': "absorb: read a local file into any set",
        'file': "src/mcp.rs",
        'find': "            let expectation = if *scope == Scope::Local {",
        'replace': "            let expectation = if true {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Every `local` file declares the same scope, so for `local` the same set is told by identity.
        'name': "absorb: never treat a local file as this very set",
        'file': "src/mcp.rs",
        'find': "                matches!((arriving.set_id, held), (Some(file), Some(held)) if file == held)",
        'replace': "                false",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A same-set link names no set, and neither does a `local` file; comparing the two would make
        # every same-set link in `local` read as a link into the absorbed set.
        'name': "absorb: let no set name match a same-set link",
        'file': "src/mcp.rs",
        'find': "            absorbed\n                .as_ref()\n                .is_some_and(|name| link.target.set() == Some(name))",
        'replace': "            link.target.set() == absorbed.as_ref()",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "absorb: tell local to drop itself",
        'file': "src/mcp.rs",
        'find': "        if declared != *scope && self.stores.contains_key(&declared) {",
        'replace': "        if self.stores.contains_key(&declared) {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A set taken on from a directory that holds another set's file -- `scatterhoard_mem` holding
        # both `scatterhoard.json` and `scatterhoard-cookbook.json` -- is found by what its header
        # declares, not by being the only file there.
        'name': "directory form: take a file whatever set it declares",
        'file': "src/mcp.rs",
        'find': "            .filter(|(_, header)| header.scope == *scope)",
        'replace': "            .filter(|(_, _header)| true)",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # A tie is not a win: a model that scores everything alike has ranked nothing first, and
        # counting ties would pass exactly the collapsed scripts the check exists to catch.
        'name': "language fit: let a tie count as ranking the paraphrase first",
        'file': "src/language_fit.rs",
        'find': ".any(|rival| cosine(anchor_vector, rival) >= target);",
        'replace': ".any(|rival| cosine(anchor_vector, rival) > target);",
        'tests': ["language_fit"],
        'expect': "caught",
    },
    {
        'name': "language fit: count the paraphrase among its own rivals",
        'file': "src/language_fit.rs",
        'find': ".filter(|(position, _)| *position != anchor && *position != paraphrase)",
        'replace': ".filter(|(position, _)| *position != anchor)",
        'tests': ["language_fit"],
        'expect': "caught",
    },
    {
        'name': "language fit: call six searchable",
        'file': "src/language_fit.rs",
        'find': "            7.. => Self::Searchable,",
        'replace': "            6.. => Self::Searchable,",
        'tests': ["language_fit"],
        'expect': "caught",
    },
    {
        'name': "language fit: call four unsearchable",
        'file': "src/language_fit.rs",
        'find': "            4..=6 => Self::Unreliable,",
        'replace': "            5..=6 => Self::Unreliable,",
        'tests': ["language_fit"],
        'expect': "caught",
    },
    {
        # The built-in Chinese is simplified; scoring a traditional-script set with it would measure
        # a script the set does not use.
        'name': "language fit: ignore the script a tag names",
        'file': "src/language_fit.rs",
        'find': "        (Some(left), Some(right)) => left == right,",
        'replace': "        (Some(_left), Some(_right)) => true,",
        'tests': ["language_fit"],
        'expect': "caught",
    },
    {
        'name': "language fit: fail to recognise a script subtag",
        'file': "src/language_fit.rs",
        'find': ".filter(|subtag| subtag.len() == 4 && subtag.chars().all(|c| c.is_ascii_alphabetic()))",
        'replace': ".filter(|subtag| subtag.len() == 5 && subtag.chars().all(|c| c.is_ascii_alphabetic()))",
        'tests': ["language_fit"],
        'expect': "caught",
    },
    {
        'name': "language fit: score a language again on every status",
        'file': "src/mcp.rs",
        'find': "                if let Some(score) = known.get(&key) {",
        'replace': "                if let Some(score) = None::<&u8> {",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "language fit: keep scores across a change of model",
        'file': "src/mcp.rs",
        'find': "        self.language_scores\n            .lock()\n            .unwrap_or_else(std::sync::PoisonError::into_inner)\n            .clear();",
        'replace': "",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "language fit: score a set its model cannot search",
        'file': "src/mcp.rs",
        'find': "        if !store\n            .can_search_by_meaning()\n            .map_err(|error| format!(\"could not read the model state: {error}\"))?\n        {\n            return Ok(fit);",
        'replace': "        if false {\n            return Ok(fit);",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "language fit: score a language but say nothing about it",
        'file': "src/mcp.rs",
        'find': "            fit.bands.extend(band.note(&about));",
        'replace': "            fit.bands.extend(None::<String>);",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # Keyword recall matches spellings, which no model takes part in.
        'name': "language fit: note a poorly served language on keyword recall too",
        'file': "src/mcp.rs",
        'find': "        if params.query.is_some() {\n            // **Only by meaning**",
        'replace': "        if true {\n            // **Only by meaning**",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "language fit: say nothing on recall",
        'file': "src/mcp.rs",
        'find': "                    notes.push(format!(\"In {}: {}\", scope.as_key(), said.join(\" \")));",
        'replace': "                    notes.push(String::new()); notes.clear(); let _ = (scope, said);",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: accept a hub listing any number of sets",
        'file': "src/store.rs",
        'find': ".filter(|(_, count)| **count != crate::language_fit::TRIPLETS_PER_LANGUAGE)",
        'replace': ".filter(|(_, count)| **count == 0)",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        # Hieroglyphs and their transliteration are different text to a model.
        'name': "model fit: count two scripts of one language together",
        'file': "src/store.rs",
        'find': "script.map_or(language.clone(), |script| format!(\"{language}-{script}\"));",
        'replace': "script.map_or(language.clone(), |_script| language.clone());",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        # American and British English are one language to a model.
        'name': "model fit: count regions of one language apart",
        'file': "src/language_fit.rs",
        'find': "    let mut subtags = tag.split('-');\n    let language = subtags.next().unwrap_or_default().to_ascii_lowercase();",
        'replace': "    let mut subtags = tag.split('-');\n    let language = tag.to_ascii_lowercase(); let _ = subtags.next();",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: let a set have two live hubs",
        'file': "src/store.rs",
        'find': "            if hub != memory.id && !replaced.contains(&hub) {",
        'replace': "            if false && hub != memory.id && !replaced.contains(&hub) {",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: accept a sentence set naming no language or several",
        'file': "src/store.rs",
        'find': "        match language_tags(memory).len() {\n            1 => {}",
        'replace': "        match 1 {\n            1 => {}",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: accept a blank sentence",
        'file': "src/language_fit.rs",
        'find': "    if triplet.iter().any(|sentence| sentence.trim().is_empty()) {",
        'replace': "    if false {",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: drop a superseded sentence set rather than follow it",
        'file': "src/store.rs",
        'find': "            for id in self.current_versions(target)? {",
        'replace': "            for id in [target] {",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: count any memory a hub links to as a sentence set",
        'file': "src/store.rs",
        'find': "                if !carries(&set, FIT_SENTENCES_MARK) {\n                    continue;\n                }\n                if let [tag]",
        'replace': "                if let [tag]",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: never use a set's own sentences",
        'file': "src/mcp.rs",
        'find': "                    theirs == language\n",
        'replace': "                    false && theirs == language\n",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: match own sentences of any script",
        'file': "src/mcp.rs",
        'find': "                        && language_fit::scripts_agree(script.as_deref(), their_script.as_deref())",
        'replace': "                        && true",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # A language scored from nine sets is on a different scale from every other.
        'name': "model fit: score a language from however many sets it has",
        'file': "src/mcp.rs",
        'find': "                    if triplets.len() != language_fit::TRIPLETS_PER_LANGUAGE {",
        'replace': "                    if triplets.is_empty() {",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: keep one score for a set's own sentences whatever they say",
        'file': "src/mcp.rs",
        'find': "                    let key = format!(\"own:{}\", triplets.concat().join(\"\\u{1f}\"));",
        'replace': "                    let key = format!(\"own:{key}\");",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: say nothing about a language with no sentences",
        'file': "src/mcp.rs",
        'find': "                        fit.setup.push(sentences_wanted(tag));",
        'replace': "                        let _ = sentences_wanted(tag);",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: put setup notes on every recall",
        'file': "src/mcp.rs",
        'find': "                let said = self.language_fit(store, &tags)?.bands;",
        'replace': "                let said: Vec<String> = { let fit = self.language_fit(store, &tags)?; fit.bands.into_iter().chain(fit.setup).collect() };",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: never report a sentence set no hub lists",
        'file': "src/store.rs",
        'find': "            if !listed.contains(&set.memory.id) {",
        'replace': "            if false && !listed.contains(&set.memory.id) {",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: say nothing of sentences for an undeclared language",
        'file': "src/mcp.rs",
        'find': "        for key in material.groups.keys().filter(|key| !matched.contains(*key)) {",
        'replace': "        for key in material.groups.keys().filter(|_key| false) {",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: accept a recommendation linking to no hub",
        'file': "src/store.rs",
        'find': "        if !carries(memory, FIT_MODEL_MARK) || self.reaches_a_hub(memory)? {",
        'replace': "        if true {",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        # Replacing the hub must not strand the recommendations linking to the old one.
        'name': "model fit: reach only the hub a recommendation names, not its replacement",
        'file': "src/store.rs",
        'find': "            if self\n                .current_versions(target)?\n                .iter()\n                .any(|id| hubs.contains(id))",
        'replace': "            if [target].iter().any(|id| hubs.contains(id))",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: list a recommendation that reaches no hub",
        'file': "src/store.rs",
        'find': "            if self.reaches_a_hub(&hit.memory)? {",
        'replace': "            if true {",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: treat an untagged recommendation as concerning no language",
        'file': "src/mcp.rs",
        'find': "                        recommendation.tags.is_empty()\n",
        'replace': "                        false\n",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: name a recommendation for any language",
        'file': "src/mcp.rs",
        'find': "                                their_language == language\n",
        'replace': "                                !their_language.is_empty()\n",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: name recommendations for a language scoring well",
        'file': "src/mcp.rs",
        'find': "            if band != Band::Searchable {",
        'replace': "            if true {",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: put recommendations on every recall",
        'file': "src/mcp.rs",
        'find': "                let said = self.language_fit(store, &tags)?.bands;",
        'replace': "                let said: Vec<String> = { let fit = self.language_fit(store, &tags)?; fit.bands.into_iter().chain(fit.advice).collect() };",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: never list recommendations",
        'file': "src/mcp.rs",
        'find': "            if !recommended.is_empty() {",
        'replace': "            if false {",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # The one place a session learns it can record a model or measure its own vocabulary.
        'name': "model fit: never say how to record a model or measure a set's vocabulary",
        'file': "src/mcp.rs",
        'find': "        if !fit.bands.is_empty() {\n            fit.advice.push(fit_how_to(&material.hubs));",
        'replace': "        if false {\n            fit.advice.push(fit_how_to(&material.hubs));",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: tell an agent to link to a hub without naming it",
        'file': "src/mcp.rs",
        'find': "        [one, ..] => format!(\"this set's `hoard-fit-hub` memory, `{one}`,\"),",
        'replace': "        [_one, ..] => \"this set's `hoard-fit-hub` memory,\".to_owned(),",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        'name': "model fit: refuse an unlinked recommendation without naming the hub",
        'file': "src/store.rs",
        'find': "        Err(StoreError::UnlinkedFitModel {\n            memory: memory.id,\n            hub,\n        })",
        'replace': "        let _ = hub;\n        Err(StoreError::UnlinkedFitModel {\n            memory: memory.id,\n            hub: None,\n        })",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: follow only bare same-set links, not ones naming this set",
        'file': "src/store.rs",
        'find': "            .filter(|link| !link.target.is_foreign_to(self.scope(), holder))",
        'replace': "            .filter(|link| holder.is_some() || matches!(link.target, LinkTarget::SameSet(_)))\n            .filter(|link| matches!(link.target, LinkTarget::SameSet(_)))",
        'tests': ["model_fit"],
        'expect': "caught",
    },
    {
        'name': "model fit: describe a score from the set's own sentences as the language's",
        'file': "src/mcp.rs",
        'find': "                    about = format!(\"{name} as this set writes it\");",
        'replace': "                    about = name.clone();",
        'tests': ["set_languages"],
        'expect': "caught",
    },
    {
        # Paging is one load; the version belongs to the session, so once.
        'name': "version: name the server version on every conventions page",
        'file': "src/mcp.rs",
        'find': "        if params.cursor == 0 {\n            result[\"server_version\"]",
        'replace': "        if true {\n            result[\"server_version\"]",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "version: never name the server version in the conventions",
        'file': "src/mcp.rs",
        'find': "            result[\"server_version\"] = json!(env!(\"CARGO_PKG_VERSION\"));",
        'replace': "            let _ = &result;",
        'tests': ["mcp"],
        'expect': "caught",
    },
    {
        'name': "version: refuse --version on the command line",
        'file': "src/cli.rs",
        'find': "        Some(\"version\" | \"--version\" | \"-V\") => Invocation::Version,",
        'replace': "",
        'tests': ["cli"],
        'expect': "caught",
    },
    {
        # `a-read-only-set-is-told-to-mint`: the note was built without reading the mode.
        'name': "create: give a read-only set the writable set's note",
        'file': "src/mcp.rs",
        'find': "        let note = match mode {",
        'replace': "        let note = match LoadMode::ReadWrite {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `author-default-from-remote-url`: the hint is where a shared set's name is asked for.
        'name': "author hint: leave the publishing-name hint out of a shared set's refusal",
        'file': "src/mcp.rs",
        'find': "they have answered. {PUBLISHING_NAME_HINT} With their answer",
        'replace': "they have answered. With their answer",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `local` is never published, so the hint would send a session to an unrelated repository.
        'name': "author hint: give local's refusal the shared set's wording",
        'file': "src/mcp.rs",
        'find': "        Scope::Local => format!(\n            \"nothing written: nobody has said who writes to {set} on this machine, and the \\",
        'replace': "        Scope::Local => format!(\n            \"{PUBLISHING_NAME_HINT} nothing written: nobody has said who writes to {set} on this machine, and the \\",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "author hint: leave the publishing-name hint out of create's note",
        'file': "src/mcp.rs",
        'find': "use `hoard_sets` action \\\"mint\\\". {PUBLISHING_NAME_HINT} Then its \\",
        'replace': "use `hoard_sets` action \\\"mint\\\". Then its \\",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The order is what worked on a small model: stop and ask, then how to mint.
        'name': "author ask: drop the instruction to stop and ask from a shared set's refusal",
        'file': "src/mcp.rs",
        'find': "published, which varies by set. Ask them for it now, and do not call `hoard_sets` until \\\n             they have answered.",
        'replace': "published, which varies by set.",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # The safeguard after the write: the human is told what was recorded.
        'name': "author relay: give a shared set nothing to relay",
        'file': "src/mcp.rs",
        'find': "        Some(relay) => format!(\"{relay} {note}\"),",
        'replace': "        Some(_) => note.to_owned(),",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "author relay: give local a sentence to relay",
        'file': "src/mcp.rs",
        'find': "        Scope::Local => None,\n    }\n}\n\n/// Where a name an action records came from",
        'replace': "        Scope::Local => Some(String::from(\"End your reply to the human.\")),\n    }\n}\n\n/// Where a name an action records came from",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "author relay: leave the relay off rename's note",
        'file': "src/mcp.rs",
        'find': "            \"note\": with_relay(\n                scope,\n                author,\n                source,\n                \"Every memory here now reads",
        'replace': "            \"note\": with_relay(\n                &Scope::Local,\n                author,\n                source,\n                \"Every memory here now reads",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `asking-the-human`: an empty `elicitation` object declares form mode, for backwards compatibility.
        'name': 'asking: treat an empty elicitation capability as no form',
        'file': "src/mcp.rs",
        'find': 'Some(modes) => modes.is_empty() || modes.contains_key("form"),',
        'replace': 'Some(modes) => modes.contains_key("form"),',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: treat URL-only elicitation as a form',
        'file': "src/mcp.rs",
        'find': 'Some(modes) => modes.is_empty() || modes.contains_key("form"),',
        'replace': 'Some(modes) => modes.is_empty() || !modes.is_empty(),',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: ask about local too',
        'file': "src/mcp.rs",
        'find': 'self.exchange.asks_human && matches!(scope, Scope::Shared(_))',
        'replace': 'self.exchange.asks_human',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        # The model writes `arguments`; only the client writes `inputResponses`.
        'name': "asking: read the human's answers from the model's arguments",
        'file': "src/mcp.rs",
        'find': '            answers: params\n                .get("inputResponses")',
        'replace': '            answers: params["arguments"]\n                .get("inputResponses")',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: let a handshake connection ask',
        'file': "src/mcp.rs",
        'find': '            asks_human: false,',
        'replace': '            asks_human: true,',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': "asking: return the handler's refusal instead of the question",
        'file': "src/mcp.rs",
        'find': '        if let Some((key, question)) = self.exchange.question.take() {',
        'replace': '        if let Some((key, question)) = self.exchange.question.take().filter(|_| false) {',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'modern: accept any version that arrives in _meta',
        'file': "src/mcp.rs",
        'find': '        if asked != MODERN_PROTOCOL {',
        'replace': '        if asked.is_empty() {',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'modern: accept a request that declares no capabilities',
        'file': "src/mcp.rs",
        'find': '            .filter(|value| value.is_object())',
        'replace': '            .or(Some(&Value::Null))',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'modern: leave resultType off a complete result',
        'file': "src/mcp.rs",
        'find': '            result["resultType"] = json!("complete");',
        'replace': '            let _ = &result;',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': "modern: serve the tool list under the handshake's revision",
        'file': "src/mcp.rs",
        'find': '                "tools": tool_definitions(MODERN_PROTOCOL),',
        'replace': '                "tools": tool_definitions(self.protocol),',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: relay a name the human typed back to them',
        'file': "src/mcp.rs",
        'find': '    if source == Source::Human {',
        'replace': '    if source == Source::Human && false {',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: ask for a mint that will be refused',
        'file': "src/mcp.rs",
        'find': '                        .map_err(|error| error.to_string())?\n                    {\n                        return Err(crate::store::StoreError::AlreadyIdentified {',
        'replace': '                        .map_err(|error| error.to_string())?\n                        .filter(|_| false)\n                    {\n                        return Err(crate::store::StoreError::AlreadyIdentified {',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: ask for a rename with nothing to rename',
        'file': "src/mcp.rs",
        'find': '                        .is_none()\n                    {\n                        return Err(crate::store::StoreError::NotIdentifiedHere.to_string());',
        'replace': '                        .is_none()\n                        && false\n                    {\n                        return Err(crate::store::StoreError::NotIdentifiedHere.to_string());',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: let the model choose the identity in a taken-on set',
        'file': "src/mcp.rs",
        'find': '        if holds_records && self.asks_about(scope) {',
        'replace': '        if holds_records && self.asks_about(scope) && false {',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: never offer to add a newcomer',
        'file': "src/mcp.rs",
        'find': '    let ask = if offer_new {',
        'replace': '    let ask = if offer_new && false {',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': "modern: require the handshake's arguments on the modern schema",
        'file': "src/mcp.rs",
        'find': '    if protocol != MODERN_PROTOCOL {\n        return branches;',
        'replace': '    if protocol == protocol {\n        return branches;',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: accept an empty name as given',
        'file': "src/mcp.rs",
        'find': '                    return match Author::new(given) {\n                        Ok(name) => Ok(Chosen::New(name)),',
        'replace': '                    return match Author::new(given).or_else(|_| Author::new("anyone")) {\n                        Ok(name) => Ok(Chosen::New(name)),',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': "asking: tell the model to mint in create's note where the human will be asked",
        'file': "src/mcp.rs",
        'find': '            LoadMode::ReadWrite | LoadMode::Evaluation if self.asks_about(&scope) => format!(',
        'replace': '            LoadMode::ReadWrite | LoadMode::Evaluation if false => format!(',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: send the model to identify in status where the human will be asked',
        'file': "src/mcp.rs",
        'find': '    if must_ask && asks {',
        'replace': '    if must_ask && asks && false {',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking: send the model to mint in status where the human will be asked',
        'file': "src/mcp.rs",
        'find': '        None if asks => format!(',
        'replace': '        None if false => format!(',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking fallback: count only a decline as unanswered',
        'file': "src/mcp.rs",
        'find': 'matches!(self.answer(key), Answer::Declined | Answer::Cancelled)',
        'replace': 'matches!(self.answer(key), Answer::Declined)',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': "asking fallback: refuse mint's name after an unanswered form",
        'file': "src/mcp.rs",
        'find': '                        return self.mint_author(&request.scope, author, Source::Model);',
        'replace': '                        let _ = author;',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking fallback: relay nothing for a name the model gave mint',
        'file': "src/mcp.rs",
        'find': '                        return self.mint_author(&request.scope, author, Source::Model);',
        'replace': '                        return self.mint_author(&request.scope, author, Source::Human);',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': "asking fallback: refuse rename's name after an unanswered form",
        'file': "src/mcp.rs",
        'find': '                        return self.rename_author(&request.scope, author, Source::Model);',
        'replace': '                        let _ = author;',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': "asking fallback: refuse identify's record after an unanswered form",
        'file': "src/mcp.rs",
        'find': '                    .filter(|_| self.asks_about(&request.scope) && unanswered)',
        'replace': '                    .filter(|_| self.asks_about(&request.scope) && unanswered && false)',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': "asking note: leave the human's answer out of the result",
        'file': "src/mcp.rs",
        'find': '            payload["author_note"] = json!(note);',
        'replace': '            let _ = note;',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking note: say nothing after a name from the form',
        'file': "src/mcp.rs",
        'find': '                if let Some(name) = from_form {',
        'replace': '                if let Some(name) = from_form.filter(|_| false) {',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        'name': 'asking note: say nothing after an identity chosen in the form',
        'file': "src/mcp.rs",
        'find': '        self.exchange.settled = Some(note);',
        'replace': '        let _ = note;',
        'tests': ['asking'],
        'expect': "caught",
    },
    {
        # `a-set-created-by-another-server-is-unreachable-until-restart`.
        'name': "rescan: never pick up a set another server created",
        'file': "src/mcp.rs",
        'find': "        self.pick_up_new_stores();\n",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "rescan: reopen a set already held",
        'file': "src/mcp.rs",
        'find': "            if self.stores.contains_key(&scope) || self.withheld.contains_key(&scope) {",
        'replace': "            if self.withheld.contains_key(&scope) {",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `a-set-dropped-by-another-server-stays-writable-here`.
        'name': "vanished: never let go of a store whose file has gone",
        'file': "src/mcp.rs",
        'find': "        self.let_go_of_vanished_stores();\n",
        'replace': "",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        'name': "vanished: compare by existence, not identity",
        'file': "src/store.rs",
        'find': "        self.file().and_then(file_identity) != Some(opened)",
        'replace': "        { let _ = opened; self.file().and_then(file_identity).is_none() }",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `vendor-prefix-scan-matches-inside-words`.
        'name': "scan: count a vendor prefix in the middle of a word",
        'file': "src/prohibited.rs",
        'find': "            if starts_a_word && run >= *minimum {",
        'replace': "            if run >= *minimum {",
        'tests': ["prohibited"],
        'expect': "caught",
    },
    {
        'name': "scan: count a prefix only at the very start of the text",
        'file': "src/prohibited.rs",
        'find': "            let starts_a_word = !text[..at].chars().next_back().is_some_and(key_character);",
        'replace': "            let starts_a_word = at == 0;",
        'tests': ["prohibited"],
        'expect': "caught",
    },
    {
        # `memory-directory-readme`.
        'name': "readme: never write the default README",
        'file': "src/mcp.rs",
        'find': "            if making_it {\n                let readme = home.join(DIRECTORY_README);",
        'replace': "            if making_it && false {\n                let readme = home.join(DIRECTORY_README);",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # Decided 2026-10-17: an existing directory, and an existing README, are the project's.
        'name': "readme: write it into a directory that already exists",
        'file': "src/mcp.rs",
        'find': "            let making_it = !home.exists();",
        'replace': "            let making_it = true;",
        'tests': ["sets"],
        'expect': "caught",
    },
    {
        # `sharing-preferences-served-at-session-start`: a person's file, once it exists, is theirs.
        'name': "preferences: write the template over an existing file",
        'file': "src/preferences.rs",
        'find': "            .create_new(true)",
        'replace': "            .create(true)\n            .truncate(true)",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "preferences: create the file but never write the template into it",
        'file': "src/preferences.rs",
        'find': ".write_all(TEMPLATE.as_bytes())",
        'replace': ".write_all(b\"\")",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # Decided 2026-10-20: the common directory, so every server on the machine reads one copy.
        'name': "preferences: keep the file in the home rather than the common directory",
        'file': "src/mcp.rs",
        'find': "        Some(Preferences::load(layout.common()))",
        'replace': "        Some(Preferences::load(layout.home()))",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "preferences: read the file and serve the catalog without it",
        'file': "src/mcp.rs",
        'find': "Some(Ok(preferences)) => self.sharing_preferences = Some(preferences.served()),",
        'replace': "Some(Ok(_)) => self.sharing_preferences = None,",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # An edit between pages would shift every cursor after it.
        'name': "preferences: read the file afresh on every page",
        'file': "src/mcp.rs",
        'find': "        if params.cursor == 0 || self.sharing_preferences.is_none() {",
        'replace': "        if true {",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # An edit takes effect at the next session start.
        'name': "preferences: keep the first session's text for every later session",
        'file': "src/mcp.rs",
        'find': "        if params.cursor == 0 || self.sharing_preferences.is_none() {",
        'replace': "        if self.sharing_preferences.is_none() {",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # After the last convention, so the catalog always arrives first and whole.
        'name': "preferences: serve them before the catalog",
        'file': "src/conventions/mod.rs",
        'find': "    format!(\"{open},\\\"sharing_preferences\\\":{sharing_preferences}}}\")",
        'replace': "    format!(\"{{\\\"sharing_preferences\\\":{sharing_preferences},{}}}\", &open[1..])",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # The human partner's addition: never served without Scatterhoard's framing.
        'name': "preferences: serve the file without its framing",
        'file': "src/preferences.rs",
        'find': "            \"framing\": FRAMING.replace(\"{file}\", &file),",
        'replace': "            \"framing\": \"\",",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # Decided 2026-10-22: the never-store list is refused at the write door, whatever the model decides.
        'name': "never-store: let a listed word through hoard_memorize",
        'file': "src/mcp.rs",
        'find': "        if let Some(refusal) = self.never_store_refusal(&written, \"Nothing was stored\") {",
        'replace': "        if let Some(refusal) = None::<String> {",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: let a listed word back in through an amendment",
        'file': "src/mcp.rs",
        'find': "        if let Some(refusal) = self.never_store_refusal(&written, \"Nothing was changed\") {",
        'replace': "        if let Some(refusal) = None::<String> {",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: leave keywords unchecked",
        'file': "src/mcp.rs",
        'find': "            context.as_ref().map_or(\"\", MemoryText::as_data),\n        ];\n        written.extend(params.keywords.iter().flatten().map(Keyword::as_str));",
        'replace': "            context.as_ref().map_or(\"\", MemoryText::as_data),\n        ];",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # "HIV" must not refuse a memory about an archive.
        'name': "never-store: match inside a longer word at its start",
        'file': "src/preferences.rs",
        'find': "                let open_before = start == 0 || !spaced(first) || !joins(folded[start - 1].0);",
        'replace': "                let open_before = true;",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: match inside a longer word at its end",
        'file': "src/preferences.rs",
        'find': "                    entry.prefix || end == folded.len() || !spaced(last) || !joins(folded[end].0);",
        'replace': "                    true;",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: ignore a trailing star",
        'file': "src/preferences.rs",
        'find': "                    entry.prefix || end == folded.len() || !spaced(last) || !joins(folded[end].0);",
        'replace': "                    end == folded.len() || !spaced(last) || !joins(folded[end].0);",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # The human partner asked for smart boundaries in the first build: Chinese may be an important case.
        'name': "never-store: treat every script as written with spaces",
        'file': "src/preferences.rs",
        'find': "    !matches!(c as u32,",
        'replace': "    true || !matches!(c as u32,",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: let Chinese text join a Latin word",
        'file': "src/preferences.rs",
        'find': "    c.is_alphanumeric() && spaced(c)",
        'replace': "    c.is_alphanumeric()",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: enforce the template's commented example",
        'file': "src/preferences.rs",
        'find': "            if line.starts_with(\"<!--\") {",
        'replace': "            if false {",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # The list ends at the next heading, so another section's items are not taken for words.
        'name': "preferences: keep the previous heading when a new one starts",
        'file': "src/preferences.rs",
        'find': "                heading = Some(text.trim_start_matches('#').trim().to_lowercase());",
        'replace': "                heading = heading.or(Some(text.trim_start_matches('#').trim().to_lowercase()));",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # The never-store list's first version: `folding-is-lowercase-not-casefold` a second time.
        'name': "never-store: lower-case entries rather than folding them",
        'file': "src/preferences.rs",
        'find': "            let folded: Vec<char> = caseless_form(stem).chars().collect();",
        'replace': "            let folded: Vec<char> = stem.chars().flat_map(char::to_lowercase).collect();",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: fold each character apart from its combining marks",
        'file': "src/preferences.rs",
        'find': "        while end < original.len() && unicode_normalization::char::is_combining_mark(original[end])",
        'replace': "        while false && unicode_normalization::char::is_combining_mark(original[end])",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # "HIV 阳性" and "HIV阳性" are both ordinary Chinese.
        'name': "never-store: require an entry's space beside Chinese",
        'file': "src/preferences.rs",
        'find': "            let optional = !spaced(before) || entry.get(k).is_some_and(|&c| !spaced(c));",
        'replace': "            let optional = false;",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: refuse a text's space beside Chinese",
        'file': "src/preferences.rs",
        'find': "            if spaced(entry[k - 1]) && spaced(entry[k]) {",
        'replace': "            if true {",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': "never-store: let a text's space split a spaced word",
        'file': "src/preferences.rs",
        'find': "            if spaced(entry[k - 1]) && spaced(entry[k]) {",
        'replace': "            if false {",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # The server applies the right list to each set; a shared-only item must not count everywhere.
        'name': "preferences: read the shared-sets list as kept out of every set",
        'file': "src/preferences.rs",
        'find': "                Some(KEEP_OUT_SHARED) => lists.keep_out_shared.push(item),",
        'replace': "                Some(KEEP_OUT_SHARED) => lists.keep_out_every.push(item),",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # Exceptions stay prose for the model and are never enforced as keep-outs.
        'name': "preferences: read exceptions as keep-outs",
        'file': "src/preferences.rs",
        'find': "                Some(EXCEPTIONS_HEADING) => lists.exceptions.push(item),",
        'replace': "                Some(EXCEPTIONS_HEADING) => lists.keep_out_every.push(item),",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # An unfamiliar list over-flags rather than going silently unenforced.
        'name': "preferences: ignore a list under an unknown heading",
        'file': "src/preferences.rs",
        'find': "                _ => lists.keep_out_every.push(item),",
        'replace': "                _ => {}",
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        # Decided 2026-10-27: the briefing comes once per review in a session, not on every page.
        'name': 'review: brief on every page',
        'file': 'src/mcp.rs',
        'find': '        let brief = full || preferences_changed.is_some() || session.briefed_in != Some(current);',
        'replace': '        let brief = true;',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: never brief a new session',
        'file': 'src/mcp.rs',
        'find': '        let brief = full || preferences_changed.is_some() || session.briefed_in != Some(current);',
        'replace': '        let brief = full || preferences_changed.is_some() || session.briefed_in.is_none();',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: do not brief on a change to the preferences',
        'file': 'src/mcp.rs',
        'find': '        let brief = full || preferences_changed.is_some() || session.briefed_in != Some(current);',
        'replace': '        let brief = full || session.briefed_in != Some(current);',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Filed and fixed 2026-10-29: each-review-page-recomputes-the-whole-set. What a page works
        # out about the whole set is kept for the next, keyed by everything it was worked out from.
        'name': 'review: keep a pre-pass answer after the memory changed',
        'file': 'src/mcp.rs',
        'find': '                Some((held, hit)) if *held == sum => *hit,',
        'replace': '                Some((_, hit)) => *hit,',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: keep pre-pass answers after the preferences changed',
        'file': 'src/mcp.rs',
        'find': '        if memo.against != against {',
        'replace': '        if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: keep the grouping after the set changed',
        'file': 'src/mcp.rs',
        'find': '            Some(held) if held.fingerprint == fingerprint => held,',
        'replace': '            Some(held) => held,',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Filed and fixed 2026-10-29: the-pre-pass-reads-a-timestamp-as-a-phone-number.
        'name': 'review: read a timestamp as a phone number',
        'file': 'src/review.rs',
        'find': '        if let Some(length) = timestamp(&chars[at..]) {',
        'replace': '        if let Some(length) = None::<usize> {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: any year makes a date',
        'file': 'src/review.rs',
        'find': '        && (1900..=2099).contains(&year)',
        'replace': '        && year > 0',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: any month makes a date',
        'file': 'src/review.rs',
        'find': '        && (1..=12).contains(&month)',
        'replace': '        && month > 0',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: any day makes a date',
        'file': 'src/review.rs',
        'find': '        && (1..=31).contains(&day);',
        'replace': '        && day > 0;',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: any digits after a date make a time',
        'file': 'src/review.rs',
        'find': '    Some(if time && (digits == 2 || digits == 4) {',
        'replace': '    Some(if time && digits > 0 {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Decided 2026-10-29: the-review-flags-the-persons-own-published-name.
        'name': 'review: flag the name the set publishes you under',
        'file': 'src/store.rs',
        'find': '        names.retain(|name| Some(name) != own.as_ref());',
        'replace': '        let _ = &own;',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: exempt every author name',
        'file': 'src/store.rs',
        'find': '        names.retain(|name| Some(name) != own.as_ref());',
        'replace': '        names.retain(|_| own.is_none());',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: exempt the name the record was minted with, not the current one',
        'file': 'src/store.rs',
        'find': '            .author_name(record)?\n            .map(|name| name.as_str().to_owned()))',
        'replace': '            .get(record)?\n            .map(|memory| memory.content.as_data().to_owned()))',
        'tests': ["store"],
        'expect': "caught",
    },
    {
        'name': 'review: do not tell the model which name is yours',
        'file': 'src/mcp.rs',
        'find': '                result["published_as"] = json!(name);',
        'replace': '                let _ = name;',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'summary: fingerprint ignores the vectors',
        'file': 'src/summary.rs',
        'find': '                .flat_map(|component| component.to_bits().to_le_bytes()),',
        'replace': '                .flat_map(|_| [0u8; 0]),',
        'tests': ["summary"],
        'expect': "caught",
    },
    {
        'name': 'summary: fingerprint ignores the keywords',
        'file': 'src/summary.rs',
        'find': '            fnv(state, keyword.as_str().bytes())',
        'replace': '            fnv(state, std::iter::empty())',
        'tests': ["summary"],
        'expect': "caught",
    },
    {
        'name': 'summary: fingerprint runs keywords together',
        'file': 'src/summary.rs',
        'find': '            let state = fnv(state, (keyword.as_str().len() as u64).to_le_bytes());',
        'replace': '            let state = state;',
        'tests': ["summary"],
        'expect': "caught",
    },
    {
        'name': 'summary: fingerprint ignores the ids',
        'file': 'src/summary.rs',
        'find': '        let state = fnv(state, subject.key.to_le_bytes());',
        'replace': '        let state = state;',
        'tests': ["summary"],
        'expect': "caught",
    },
    {
        # Decided 2026-10-22: exact values the person lists are not put to them in every memory.
        'name': 'review: let always-fine silence nothing',
        'file': 'src/review.rs',
        'find': '        .filter(|(_, matched)| always_fine.find(matched).is_none())',
        'replace': '        .filter(|_| true)',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: flag /Users/Shared as a home path',
        'file': 'src/review.rs',
        'find': '                if !user.is_empty() && !user.eq_ignore_ascii_case("shared") {',
        'replace': '                if !user.is_empty() {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # A first label with no letters, as in `~/.local` or a bare IP, is not a host name.
        'name': 'review: take the .local directory for an internal host',
        'file': 'src/review.rs',
        'find': '            && labels[0].chars().any(|c| c.is_ascii_alphabetic())',
        'replace': '            && true',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Each false positive costs the person a glance; dates and ids must pass.
        'name': 'review: read a date as a phone number',
        'file': 'src/review.rs',
        'find': '            && (10..=15).contains(&digits);',
        'replace': '            && (8..=15).contains(&digits);',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # local is for memories about the person.
        'name': 'review: flag a local memory for being about the person',
        'file': 'src/review.rs',
        'find': '                if !personal {',
        'replace': '                if true {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Decided 2026-10-25: a report about someone else clears, a judgement never does.
        'name': 'review: clear a judgement about someone else',
        'file': 'src/review.rs',
        'find': '            About::SomeoneElse if judges => {',
        'replace': '            About::SomeoneElse if false => {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: clear a judgement about the person in local',
        'file': 'src/review.rs',
        'find': '                if judges {',
        'replace': '                if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # A sentence about a person with no answer for what it says is read as judging.
        'name': 'review: read an unanswered report-or-judgement as a report',
        'file': 'src/review.rs',
        'find': '        let judges = answer.says != Some(Says::Reports);',
        'replace': '        let judges = answer.says == Some(Says::Judges);',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # The person's keep-outs govern what they write about others, never loosening the template's.
        'name': 'review: check someone else against the person\'s own lists',
        'file': 'src/review.rs',
        'find': '        let applying = if answer.about == About::SomeoneElse {',
        'replace': '        let applying = if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'preferences: leave the template\'s every-set items out for others',
        'file': 'src/preferences.rs',
        'find': '        for item in template.keep_out_every.iter().chain(&self.keep_out_every) {',
        'replace': '        for item in self.keep_out_every.iter() {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'preferences: leave the template\'s shared-only items out for others',
        'file': 'src/preferences.rs',
        'find': '        for item in template.keep_out_shared.iter().chain(&self.keep_out_shared) {',
        'replace': '        for item in self.keep_out_shared.iter() {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'preferences: leave the person\'s every-set items out for others',
        'file': 'src/preferences.rs',
        'find': '        for item in template.keep_out_every.iter().chain(&self.keep_out_every) {',
        'replace': '        for item in template.keep_out_every.iter() {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # `a-preference-written-without-a-bullet-is-dropped`: a line the parser does not read is
        # reported, never dropped in silence.
        'name': 'preferences: drop a bare line without reporting it',
        'file': 'src/preferences.rs',
        'find': '                lists.unread.push(unread(false));',
        'replace': '',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: leave an item inside a comment unreported',
        'file': 'src/preferences.rs',
        'find': '                    lists.unread.push(unread(true));',
        'replace': '',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: report the template\'s own examples inside a comment',
        'file': 'src/preferences.rs',
        'find': '!item.is_empty() && !examples.contains(&item)',
        'replace': '!item.is_empty()',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: report an empty marker inside a comment',
        'file': 'src/preferences.rs',
        'find': '!item.is_empty() && !examples.contains(&item)',
        'replace': '!examples.contains(&item)',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: report blank lines',
        'file': 'src/preferences.rs',
        'find': '            if line.is_empty() {\n                continue;\n            }\n',
        'replace': '',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: report an empty item',
        'file': 'src/preferences.rs',
        'find': '    if matches!(line, "-" | "*" | "+") {\n        return Some("");\n    }\n',
        'replace': '',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: stop reading a plus as an item marker',
        'file': 'src/preferences.rs',
        'find': '    ["- ", "* ", "+ "]\n        .iter()',
        'replace': '    ["- ", "* "]\n        .iter()',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: count reported lines from zero',
        'file': 'src/preferences.rs',
        'find': '                line: index + 1,',
        'replace': '                line: index,',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: find no examples in the template',
        'file': 'src/preferences.rs',
        'find': '            examples.extend(item_of(line).filter(|item| !item.is_empty()));',
        'replace': '',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: serve the report even when nothing is unread',
        'file': 'src/preferences.rs',
        'find': '        if !unread.is_empty() {\n            served["not_read"]',
        'replace': '        if true {\n            served["not_read"]',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'preferences: serve the unread lines without the note',
        'file': 'src/preferences.rs',
        'find': '            served["not_read_note"] = json!(NOT_READ_NOTE);',
        'replace': '',
        'tests': ["sharing_preferences"],
        'expect': "caught",
    },
    {
        'name': 'review: serve only the person\'s lists for others',
        'file': 'src/mcp.rs',
        'find': '            result["others_keep_out"] = json!(lists.for_others().items());',
        'replace': '            result["others_keep_out"] = json!(lists.items());',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Found 2026-10-26: a flagged memory amended since it was shown was cleared by the person.
        'name': 'review: let the person clear a flagged memory that changed since',
        'file': 'src/mcp.rs',
        'find': '            if current.is_none() || current != shown {',
        'replace': '            if current.is_none() {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: keep a forgotten memory awaiting the person',
        'file': 'src/mcp.rs',
        'find': '        self.prune_flagged(scope)?;\n        let session = self.reviews.entry(scope.clone()).or_default();',
        'replace': '        let session = self.reviews.entry(scope.clone()).or_default();',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: skip pruning before the person clears',
        'file': 'src/mcp.rs',
        'find': '        self.refuse_if_read_only(scope)?;\n        self.prune_flagged(scope)?;',
        'replace': '        self.refuse_if_read_only(scope)?;',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # The server applies the lists; a small model did not.
        'name': 'review: count a shared-only item in local',
        'file': 'src/review.rs',
        'find': '            if personal && shared_only && !kept_everywhere {',
        'replace': '            if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # A small model paraphrased "where I live, and how to contact me".
        'name': 'review: match quoted items exactly',
        'file': 'src/review.rs',
        'find': '        && (said == listed || listed.starts_with(&format!("{said} ")) || said.starts_with(&listed))',
        'replace': '        && (said == listed || said.starts_with(&listed))',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # An export publishes the reasoning and context as surely as the text.
        'name': "review: number only a memory's text",
        'file': 'src/review.rs',
        'find': '        ("why", why),\n        ("context", context),\n    ] {',
        'replace': '    ] {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # A small model stops early unless completeness is required.
        'name': 'clear: accept an answer missing a sentence',
        'file': 'src/mcp.rs',
        'find': '            if (1..=*count).any(|n| !answered.contains(&n)) {',
        'replace': '            if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'clear: accept an answer for a memory changed since it was shown',
        'file': 'src/mcp.rs',
        'find': '            if sum != *shown_sum {',
        'replace': '            if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'clear: let the person clear what was never put to them',
        'file': 'src/mcp.rs',
        'find': '            if !session.flagged.contains(id) {',
        'replace': '            if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Decided 2026-10-22: the file on disk is the clearance record.
        'name': 'export: write unreviewed memories',
        'file': 'src/mcp.rs',
        'find': '        if let Some(refusal) = self.export_review_refusal(scope, destination)? {',
        'replace': '        if let Some(refusal) = None::<String> {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'backup: write an unmarked header',
        'file': 'src/mcp.rs',
        'find': '        memory_file::write_header_marked(scope, set_id, true, &mut body)',
        'replace': '        memory_file::write_header_marked(scope, set_id, false, &mut body)',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # A backup was never reviewed, wherever it is put.
        'name': "review: count a backup's memories as reviewed",
        'file': 'src/mcp.rs',
        'find': '        if header.backup {',
        'replace': '        if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'session start: remind about published shared sets too',
        'file': 'src/mcp.rs',
        'find': '            if Self::review_kind(scope, &lists) == SetKind::Shared {',
        'replace': '            if false {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': 'review: reopen nothing for a new never-store word',
        'file': 'src/mcp.rs',
        'find': '                words.find(&text).is_some()',
        'replace': '                false',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        'name': "review: ignore a full review's request",
        'file': 'src/mcp.rs',
        'find': '            if full || became_public {',
        'replace': '            if became_public {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Going public lapses every "while it is not public" exception.
        'name': 'review: ignore a project turning public',
        'file': 'src/mcp.rs',
        'find': '            if full || became_public {',
        'replace': '            if full {',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # An identity record is the name a set publishes under, put there on purpose.
        'name': 'review: put identity records up for review',
        'file': 'src/mcp.rs',
        'find': '                if let memory_file::Record::Memory(memory) = record\n                    && memory.created.by != memory.id\n                {\n                    let sum',
        'replace': '                if let memory_file::Record::Memory(memory) = record\n                    && true\n                {\n                    let sum',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # An SSH remote sends a memory to the person for a string that names no one.
        'name': 'pre-pass: read an SSH remote as an email address',
        'file': 'src/review.rs',
        'find': '        if start < at\n            && !ssh_remote',
        'replace': '        if start < at\n            && true',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # Without the `:path`, nothing says `git@host` is a remote rather than an address.
        'name': 'pre-pass: exempt any address whose user is git',
        'file': 'src/review.rs',
        'find': "            && chars.get(end) == Some(&':')\n",
        'replace': '',
        'tests': ["export_review"],
        'expect': "caught",
    },
    {
        # A pseudonym left out of the preferences is flagged again by every later review.
        'name': 'review: stop offering a new pseudonym to the always-fine list',
        'file': 'src/mcp.rs',
        'find': '                 If a rewording replaces a name with a pseudonym, suggest the person add the \\\n                 pseudonym under *Always fine to share* in their sharing preferences, so later \\\n                 reviews do not flag it."',
        'replace': '"',
        'tests': ["export_review"],
        'expect': "caught",
    },
]
