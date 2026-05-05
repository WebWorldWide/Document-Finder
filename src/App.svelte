<script lang="ts">
  import Sidebar from "./components/Sidebar.svelte";
  import FindTab from "./components/FindTab.svelte";
  import LibraryView from "./components/LibraryView.svelte";
  import SettingsView from "./components/SettingsView.svelte";
  import { uiStore } from "./stores/ui.svelte";
</script>

<div class="flex h-screen w-screen overflow-hidden bg-[var(--color-background)] text-[var(--color-foreground)]">
  <Sidebar />
  <main class="flex flex-1 flex-col overflow-hidden">
    <div
      class="h-8 shrink-0 border-b border-[var(--color-border)]"
      data-tauri-drag-region
    ></div>
    <div class="flex-1 overflow-auto p-6">
      {#if uiStore.view === "find"}
        <FindTab />
      {:else if uiStore.view === "library"}
        <LibraryView />
      {:else if uiStore.view === "settings"}
        <SettingsView />
      {/if}
    </div>
  </main>
</div>
