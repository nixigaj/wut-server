<img align="left" alt="wut icon" src="icon.svg" height="128" style="margin-right: 1rem" />

# `wut` server
This is a simple HTTP(/2) server written in Rust,
made for one thing, and one thing only: securely echoing your public IP-address as fast as possible. It is made for the [`wut` cli tool](https://github.com/nixigaj/wut), and I use it for [ip.erix.dev:11313](http://ip.erix.dev:11313).

## Non-scientific comparison with Nginx
### Server
| CPU                        | RAM      | Uplink   |
|----------------------------|----------|----------|
| Single core AMD EPYC 7302P | 1 GB ECC | 1 Gbit/s |

### Client
| CPU            | RAM       | Uplink     |
|----------------|-----------|------------|
| AMD Ryzen 5600 | 32 GB ECC | 250 Mbit/s |

The tool used for the benchmark is [wrk](https://github.com/wg/wrk). The bottleneck in this test will be the server CPU.

### Nginx
Nginx has all optimization options enabled, headers stripped with the [ngx_headers_more module](https://github.com/openresty/headers-more-nginx-module), and is compiled with static OpenSSL and LTO.

#### Config
```nginx
load_module modules/ngx_http_headers_more_filter_module.so;

pid /var/run/nginx.pid;

worker_processes 1;

events {
	worker_connections 1024;
}

http {
	sendfile on;
	server_tokens off;
	more_clear_headers "Server";
	more_clear_headers "X-Powered-By";
	more_clear_headers "content-type";

	http2 on;
	ssl_protocols TLSv1.3;
	ssl_early_data on;
	ssl_certificate <cert path>;
	ssl_certificate_key <key path>;

	server {
		return 200 "$remote_addr";
		listen [<IPv6>]:443 ssl;
		listen <IPv4>:443 ssl;
	}
}
```

#### Benchmark
```
Running 10s test @ https://ip.erix.dev
  6 threads and 1000 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency    28.07ms   10.63ms 520.46ms   86.30%
    Req/Sec     4.36k     0.96k    6.54k    72.18%
  257322 requests in 10.03s, 27.98MB read
  Socket errors: connect 95, read 17, write 0, timeout 0
Requests/sec:  25650.31
Transfer/sec:      2.79MB
```

### `wut` server

#### Benchmark
```
Running 10s test @ https://ip.erix.dev:11313
  6 threads and 1000 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency    45.00ms   50.48ms 279.57ms   78.59%
    Req/Sec     6.41k     1.23k    8.65k    83.47%
  382057 requests in 10.02s, 32.79MB read
Requests/sec:  38110.60
Transfer/sec:      3.27MB
```

That is about 49% more requests per second. Additionally, some socket errors were encountered with Nginx.

## Usage
```
Usage: wut-server [OPTIONS] --cert-path <CERT_PATH> --key-path <KEY_PATH>

Options:
  -b, --bind <BIND>                  Address to bind to, with optional port (can be provided multiple times) [default: 127.0.0.1:11313 [::1]:11313]
  -c, --cert-path <CERT_PATH>        Certificate file path
  -k, --key-path <KEY_PATH>          Key file path
  -i, --log-interval <LOG_INTERVAL>  Log interval in seconds [default: 60]
  -2, --http2-only                   Use HTTP/2 only
  -h, --help                         Print help
  -V, --version                      Print version
```

## Planned features

- Automatic hot reload of certificates when files on disk change.

## License
All files in this repository are licensed under the [MIT License](LICENSE).

The icon is a reference to the [Confused Nick Young / Swaggy P](https://knowyourmeme.com/memes/confused-nick-young-swaggy-p) meme.
