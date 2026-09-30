# punctuality-ingest

The scheduled producer for [`capabilities/punctuality`](../punctuality/). It runs the
punctuality CLI once per day and lets the capability's ingest ledger decide whether to
merge a newly published month or rebuild after an upstream correction.

The job is separate because `punctuality` is an always-on HTTP service and Sjel refuses a
manifest that declares both `autostart` and `schedule`. The punctuality capability remains
the owner of the aggregate, schema, and statistics contract; this manifest only schedules
its producer.
