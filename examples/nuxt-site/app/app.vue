<template>
  <nav>
    <NuxtLink :to="$localeRoute('/')">{{ $t('nav.home') }}</NuxtLink>
    <NuxtLink :to="$localeRoute('/settings')">{{ $t('nav.settings') }}</NuxtLink>
    <NuxtLink :to="$localeRoute('/about')">{{ $t('nav.about') }}</NuxtLink>
    <NuxtLink v-for="l in $getLocales()" :key="l.code" :to="$switchLocaleRoute(l.code)">{{ l.displayName }}</NuxtLink>
  </nav>
  <NuxtPage />
</template>
