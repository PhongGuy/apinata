#!/bin/sh
# Test-only PKI for the network tests; none of these keys protects anything. Run from this
# directory; it rewrites every file it lists. The passphrase of the encrypted keys is `secret`.
set -e
DAYS=36500
openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -days $DAYS -subj "/CN=apinata-test-ca" </dev/null
openssl req -newkey rsa:2048 -nodes -keyout server.key -out server.csr -subj "/CN=localhost" </dev/null
printf "basicConstraints=CA:FALSE\nsubjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n" > server.ext
openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out server.pem -days $DAYS -extfile server.ext </dev/null
openssl req -newkey rsa:2048 -nodes -keyout client.key -out client.csr -subj "/CN=client" </dev/null
printf "basicConstraints=CA:FALSE\nextendedKeyUsage=clientAuth\n" > client.ext
openssl x509 -req -in client.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out client.pem -days $DAYS -extfile client.ext </dev/null
openssl x509 -req -in client.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out expired-client.pem -extfile client.ext -not_before 20190101000000Z -not_after 20200101000000Z </dev/null
openssl pkcs8 -topk8 -in client.key -out client-enc.key -passout pass:secret
openssl rsa -in client.key -aes256 -traditional -out client-trad.key -passout pass:secret
# A server certificate that expired in 2020 under its own CA, so a client trusting that CA fails
# on expiry alone. Not signed by ca.pem, whose key is gone by now.
openssl req -x509 -newkey rsa:2048 -nodes -keyout expired-ca.key -out expired-ca.pem -days $DAYS -subj "/CN=apinata-expired-test-ca" </dev/null
openssl req -newkey rsa:2048 -nodes -keyout expired-server.key -out expired-server.csr -subj "/CN=localhost" </dev/null
printf "basicConstraints=CA:FALSE\nsubjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n" > expired-server.ext
openssl x509 -req -in expired-server.csr -CA expired-ca.pem -CAkey expired-ca.key -CAcreateserial -out expired-server.pem -extfile expired-server.ext -not_before 20190101000000Z -not_after 20200101000000Z </dev/null
# A self-signed server certificate that is also a CA (basicConstraints CA:TRUE, the openssl
# default), which rustls rejects as CaUsedAsEndEntity even when it is the only trust anchor.
openssl req -x509 -newkey rsa:2048 -nodes -keyout selfsigned-ca-leaf.key -out selfsigned-ca-leaf.pem -days $DAYS -subj /CN=localhost -addext basicConstraints=critical,CA:TRUE -addext subjectAltName=DNS:localhost,IP:127.0.0.1 </dev/null
rm -f expired-ca.key expired-ca.srl expired-server.csr expired-server.ext ca.key ca.srl server.csr client.csr server.ext client.ext
