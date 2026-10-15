<script lang="ts">
  // The media ledger's read-only surface. Everything on this page comes from
  // capabilities/media over HTTP; the page owns no ledger state and writes nothing.
  import PageHeader from "$lib/PageHeader.svelte";
  import MediaLedger from "$lib/media/MediaLedger.svelte";
</script>

<PageHeader
  badge="Media"
  title="Media ledger"
  desc="What the ingest gate knows: how many digests and paths are indexed, which volumes hold them, and what each ingest run did with every path it considered."
/>

<MediaLedger />
