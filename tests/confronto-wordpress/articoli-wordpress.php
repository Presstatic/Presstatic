<?php
// 2.000 articoli di circa 5 KB, in 6 categorie, come nel sito di prova di Presstatic.
wp_defer_term_counting(true); wp_suspend_cache_addition(true);
$words = explode(' ', 'comune sindaco lavori strada ponte fiume scuola ospedale piazza mercato calcio squadra partita cultura mostra teatro ambiente parco raccolta rifiuti regione bilancio consiglio cittadini quartiere stazione treno autobus estate inverno');
$cats = [];
foreach (['Cronaca','Politica','Economia','Sport','Cultura','Ambiente'] as $c) { $t = wp_insert_term($c, 'category'); $cats[] = is_wp_error($t) ? get_term_by('name', $c, 'category')->term_id : $t['term_id']; }
mt_srand(7);
$text = function($n) use ($words) { $o = []; for ($i = 0; $i < $n; $i++) $o[] = $words[mt_rand(0, count($words) - 1)]; return implode(' ', $o); };
for ($i = 1; $i <= 2000; $i++) {
  $body = '';
  for ($p = 0; $p < 6; $p++) { $body .= '<!-- wp:paragraph --><p>' . $text(80) . '</p><!-- /wp:paragraph -->'; if ($p == 2) $body .= '<!-- wp:heading --><h2 class="wp-block-heading">' . $text(5) . '</h2><!-- /wp:heading -->'; }
  wp_insert_post(['post_title' => ucfirst($text(10)), 'post_name' => "articolo-$i", 'post_excerpt' => $text(22), 'post_content' => $body, 'post_status' => 'publish',
    'post_date' => date('Y-m-d H:i:s', time() - (2000 - $i) * 300), 'post_category' => [$cats[$i % 6]], 'tags_input' => ['argomento ' . ($i % 300), 'argomento ' . (($i * 7) % 300)]]);
}
wp_defer_term_counting(false);
echo "articoli: ", wp_count_posts()->publish, "\n";
