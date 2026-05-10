<?php
// OPcache preload: pre-compila e indexa server.php sem startar o servidor.
// Roda 1× no boot do CLI antes do worker. Custo runtime: zero.

declare(strict_types=1);

define('PRELOADING', true);

require '/app/server.php';
