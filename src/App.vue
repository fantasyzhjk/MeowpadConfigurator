<script setup lang="ts">
// This starter template is using Vue 3 <script setup> SFCs
// Check out https://vuejs.org/api/sfc-script-setup.html#script-setup
import Main from '@/components/Main.vue'
import Application from './components/Application.vue';
import { darkTheme } from "naive-ui";
import { invoke } from "@tauri-apps/api/tauri";
import { appWindow, Theme } from "@tauri-apps/api/window";
import { NConfigProvider, GlobalThemeOverrides } from 'naive-ui'
import { IVersion } from '@/interface';
import { useStore } from '@/store'

const lightThemeOverrides: GlobalThemeOverrides = {
  Layout: {
    color: '#F7F7F7',
    headerColor: '#F7F7F7',
  }
}

const store = useStore()
const theme = ref<Theme | null>(null)

// 禁用webkit右键菜单
document.body.onselectstart = document.body.oncontextmenu = () => false

onMounted(async () => {
  let show = true;

  invoke("get_version").then((version: IVersion) => {
    store.version_info = version
    invoke("check_update", { version: version }).then((res: boolean) => {
      show = res;
    });
  });

  theme.value = await appWindow.theme()
  await appWindow.onThemeChanged(({ payload: t }) => {
    theme.value = t
  })

  setTimeout(async () => {
    if (show) {
      await appWindow.show()
    }
  }, 500)
})
</script>

<template>
  <!-- <n-theme-editor> -->
  <n-config-provider :theme="theme == 'dark' ? darkTheme : undefined"
    :theme-overrides="theme == 'dark' ? null : lightThemeOverrides">
    <Application>
      <n-layout>
        <n-layout-header class="grid grid-flow-col justify-items-stretch header">
          <Header />
        </n-layout-header>
        <n-layout-content class="main">
          <div class="flex justify-center items-center h-full" id="main">
            <Main />
          </div>
        </n-layout-content>
      </n-layout>
    </Application>
  </n-config-provider>
  <!-- </n-theme-editor> -->
</template>

<style lang="scss">
$header-height: 56px;

.header {
  height: $header-height;
}

.main {
  height: calc(100vh - $header-height);
}
</style>

