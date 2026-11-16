<template>
  <h1>{{ $t('title') }}</h1>
  <p>{{ $t('body', { brand: $t('brand') }) }}</p>
  <p>{{ $t('source') }}</p>
</template>
