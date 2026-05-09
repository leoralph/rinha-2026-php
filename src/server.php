<?php

declare(strict_types=1);

$sock = getenv('SOCK') ?: '/run/sock/api1.sock';

if (file_exists($sock)) {
    @unlink($sock);
}

$server = new Swoole\Http\Server($sock, 0, SWOOLE_BASE, SWOOLE_SOCK_UNIX_STREAM);

$server->set([
    'worker_num'           => 1,
    'reactor_num'          => 1,
    'tcp_fastopen'         => true,
    'open_tcp_nodelay'     => true,
    'enable_coroutine'     => false,
    'log_level'            => SWOOLE_LOG_WARNING,
    'http_compression'     => false,
    'send_yield'           => false,
    'open_http2_protocol'  => false,
    'max_request'          => 0,
    'max_conn'             => 4096,
    'buffer_output_size'   => 2 * 1024 * 1024,
]);

// Pre-rendered respostas (count fraudes em top-5 → string JSON).
const FRAUD_BODIES = [
    '{"approved":true,"fraud_score":0}',
    '{"approved":true,"fraud_score":0.2}',
    '{"approved":true,"fraud_score":0.4}',
    '{"approved":false,"fraud_score":0.6}',
    '{"approved":false,"fraud_score":0.8}',
    '{"approved":false,"fraud_score":1}',
];

$server->on('start', function () use ($sock) {
    @chmod($sock, 0666);
});

$server->on('request', function (Swoole\Http\Request $req, Swoole\Http\Response $resp) {
    $uri = $req->server['request_uri'] ?? '';

    if ($uri === '/fraud-score') {
        // Default Content-Type já é text/html; k6 só checa status + body.
        $resp->end(FRAUD_BODIES[\rinha_fraud_count($req->rawContent() ?: '{}')]);
        return;
    }

    if ($uri === '/ready') {
        $resp->end('');
        return;
    }

    $resp->status(404);
    $resp->end('');
});

$server->start();
