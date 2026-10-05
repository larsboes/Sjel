<script lang="ts">
  // `/inventory` ist die Adresse der Capability, nicht eine zweite Seite.
  //
  // Die Oberflaeche fuer die Zeilen liegt heute unter `/interior`, weil sie dort entstanden
  // ist, als das Inventar noch zu `interior` gehoerte (ISA F13). Die Capability ist am
  // 2026-10-05 ausgezogen, die Seite noch nicht — und eine zweite Liste danebenzustellen waere
  // die Drift, gegen die dieses Projekt existiert. Also zeigt `/inventory` auf die eine Seite,
  // die es gibt, statt eine leere anzubieten.
  //
  // Was noch fehlt: die Zeilen-Oberflaeche (Liste, Wunschliste, Anlegen, Bearbeiten) nach
  // `/inventory` ziehen und unter `/interior` nur den Grundriss lassen. Dann dreht sich dieser
  // Stub um.
  import { link } from "$lib/nav";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  onMount(() => {
    void goto(link("/interior"), { replaceState: true });
  });
</script>

<p>Was du besitzt, steht unter <a href={link("/interior")}>Interior</a>.</p>
