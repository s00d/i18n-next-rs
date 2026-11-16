<template>
  <h1>{{ $t('title') }}</h1>
  <p>{{ $t('language') }}: {{ $i18n.getLocale() }}</p>
  <p>{{ $tn(0.256, { style: 'percent' }) }} · {{ $td(new Date(), 'short') }}</p>
</template>
