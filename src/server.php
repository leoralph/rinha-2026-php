<?php

declare(strict_types=1);

function build_server(string $sock): \Swoole\Server {
    if (file_exists($sock)) {
        @unlink($sock);
    }

    $server = new \Swoole\Server($sock, 0, SWOOLE_BASE, SWOOLE_SOCK_UNIX_STREAM);

    $server->set([
        'worker_num'                       => 1,
        'reactor_num'                      => 1,
        'tcp_fastopen'                     => true,
        'open_tcp_nodelay'                 => true,
        'enable_coroutine'                 => false,
        'log_level'                        => SWOOLE_LOG_WARNING,
        'open_eof_check'                   => false,
        'open_length_check'                => false,
        'max_request'                      => 0,
        'max_conn'                         => 4096,
        'buffer_output_size'               => 2 * 1024 * 1024,
        'kernel_socket_recv_buffer_size'   => 65536,
        'kernel_socket_send_buffer_size'   => 65536,
    ]);

    $server->on('start', static function () use ($sock) {
        @chmod($sock, 0666);
    });

    // Handler RAW: cada `receive` recebe bytes da request HTTP. Rust parsea,
    // computa, retorna response HTTP completa. PHP só faz forwarding.
    $server->on('receive', static function (\Swoole\Server $srv, int $fd, int $rid, string $data): void {
        $srv->send($fd, \rinha_handle($data));
    });

    return $server;
}

if (!defined('PRELOADING')) {
    $sock = getenv('SOCK') ?: '/run/sock/api1.sock';
    build_server($sock)->start();
}
