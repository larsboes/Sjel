<script lang="ts">
  // `/inventory` is the rows' surface: the interior page opened on its inventory view, so the
  // list, the wishlist, the editor and the offline sync stay one implementation. The floor plan
  // lives at `/interior` (under More). Replaces the 2026-10-05 redirect stub (b10e5260).
  //
  // ponytail: the rows' UI still sits inside routes/interior/+page.svelte. Extract it into
  // $lib/inventory when the interior page's plan views and the rows stop sharing editor state.
  import InteriorPage from "../interior/+page.svelte";
</script>

<InteriorPage initialView="inventory" />
