<script>
  import { enhance } from '$app/forms';
  import { user } from '$lib/stores';
</script>

<nav>
  <a href="/">{$user.name}</a>
  <form method="POST" action="/logout" use:enhance></form>
</nav>
