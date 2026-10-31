<script lang="ts">
  import { formatDate } from '$lib/format';
  let { data } = $props();
  let shown = $state(10);
  function more() {
    shown += 10;
  }
</script>

{#each data.items.slice(0, shown) as item}
  <p>{formatDate(item.date)}</p>
{/each}
<button onclick={more}>More</button>
