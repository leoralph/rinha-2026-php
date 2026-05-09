<?php

declare(strict_types=1);

ignore_user_abort(true);

// Pre-warm: mmap dos 3 arquivos + parse das tabelas embedadas no .so.
\rinha_warmup();

$handler = static function (): void {
    $method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
    $path = $_SERVER['REQUEST_URI'] ?? '/';

    if ($method === 'GET' && $path === '/ready') {
        return;
    }

    if ($method === 'POST' && $path === '/fraud-score') {
        $body = file_get_contents('php://input');
        header('Content-Type: application/json');
        echo \rinha_score($body !== false ? $body : '');
        return;
    }

    http_response_code(404);
};

while (\frankenphp_handle_request($handler)) {
    // Worker mode: cada iteração processa uma request HTTP.
}
