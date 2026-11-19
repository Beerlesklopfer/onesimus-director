#!/bin/sh
# Server 2: Datei-Konfiguration (slapd.conf, slapd -f), Debian-Standard-ACL (root/EXTERNAL ohne userPassword)
set -e
F=/opt/ldaptest
mkdir -p /etc/ldap/schema/local /etc/ldap/tls /srv/ldap/main /srv/ldap/zweit
cp $F/slapd.conf /etc/ldap/slapd.conf; cp $F/onesimus.schema /etc/ldap/schema/local/
cd /etc/ldap/tls
openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.crt -subj /CN=ldapcheck-ca -days 30 2>/dev/null
openssl req -newkey rsa:2048 -nodes -keyout server.key -out server.csr -subj /CN=conf -addext subjectAltName=DNS:conf,DNS:localhost 2>/dev/null
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -copy_extensions copy -out server.crt -days 30 2>/dev/null
chown root:openldap server.key; chmod 640 server.key
chown -R openldap:openldap /srv/ldap; chmod 700 /srv/ldap/main /srv/ldap/zweit
chown root:openldap /etc/ldap/slapd.conf; chmod 640 /etc/ldap/slapd.conf
# O-7 (--network host) sets LDAPTEST_SLAPD_SERVICES=ldapi:/// (no TCP port in the host's namespace)
services=${LDAPTEST_SLAPD_SERVICES:-ldap:/// ldaps:/// ldapi:///}
sed -i "s|^SLAPD_CONF=.*|SLAPD_CONF=/etc/ldap/slapd.conf|; s|^SLAPD_SERVICES=.*|SLAPD_SERVICES=\"$services\"|" /etc/default/slapd
install -d -o openldap -g openldap /run/slapd
slaptest -u -f /etc/ldap/slapd.conf >/dev/null 2>&1
ldap-ctl start
A="-x -D cn=admin,dc=example,dc=com -w admin123 -H ldapi:///"
ldapadd $A -f $F/data-com.ldif >/dev/null
ldapadd $A -f $F/special-com.ldif >/dev/null
ldapadd -x -D cn=admin,o=zweit,c=de -w admin123 -H ldapi:/// -f $F/zweit.ldif >/dev/null
echo "conf ready"
