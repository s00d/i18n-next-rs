<!-- The texts of playground-rust `page_texts("index")`, key for key (data-k). -->
<template>
  <main>
    <h1 data-k="nav.home">{{ $t('nav.home') }}</h1>
    <p data-k="greeting">{{ $t('greeting', { name: 'Ann' }) }}</p>
    <p data-k="apples">{{ $tc('apples', 3) }}</p>
    <p data-k="cart">{{ $tc('cart', 0) }}</p>
    <p data-k="demo.price">{{ $t('demo.price', { price: $tn(1234.5) }) }}</p>
    <p data-k="footer">{{ $t('footer') }}</p>
    <p data-k="shout">{{ $t('shout') }}</p>
  </main>
</template>
