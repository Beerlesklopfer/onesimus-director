#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Server 1: Online-Konfiguration (cn=config, /etc/ldap/slapd.d), Debian-Standard plus Testinhalt
set -e
F=/opt/ldaptest
cat <<X | debconf-set-selections
slapd slapd/no_configuration boolean false
slapd slapd/domain string example.org
slapd shared/organization string Example
slapd slapd/password1 password admin123
slapd slapd/password2 password admin123
slapd slapd/purge_database boolean true
slapd slapd/move_old_database boolean false
X
DEBIAN_FRONTEND=noninteractive dpkg-reconfigure slapd >/dev/null 2>&1
# O-7 (--network host): only ldapi, no TCP port 389 in the host's namespace
if [ -n "${LDAPTEST_SLAPD_SERVICES:-}" ]; then
  sed -i "s|^SLAPD_SERVICES=.*|SLAPD_SERVICES=\"$LDAPTEST_SLAPD_SERVICES\"|" /etc/default/slapd
fi
ldap-ctl start
E="-Q -Y EXTERNAL -H ldapi:///"
ldapadd $E -f /etc/ldap/schema/cosine.ldif >/dev/null 2>&1 || true
install -d -o openldap -g openldap -m 700 /var/lib/ldap/second
ldapmodify $E -f $F/olc-setup.ldif >/dev/null
ldapmodify $E -f $F/olc-schema.ldif >/dev/null
ldapmodify $E -f $F/proxy.ldif >/dev/null
ldapadd -x -D cn=admin,dc=example,dc=org -w admin123 -H ldapi:/// -f $F/data.ldif >/dev/null
ldapadd -x -D cn=admin,dc=example,dc=org -w admin123 -H ldapi:/// -f $F/special.ldif >/dev/null
ldapadd -x -D cn=admin,o=second,c=de -w admin123 -H ldapi:/// -f $F/second.ldif >/dev/null
echo "olc ready"
