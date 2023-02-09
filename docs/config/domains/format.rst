This file contains the list authentication realms.

Each user configuration section starts with the header ``<realm-type>: <name>``,
followed by the realm's configuration options.

For LDAP realms, the LDAP bind password is stored in ``ldap_passwords.json``.

::

  openid: master
	client-id pbs
	comment
	issuer-url http://192.168.0.10:8080/realms/master
	username-claim username

  ldap: ldap-server
	base-dn OU=People,DC=ldap-server,DC=example,DC=com
	mode ldaps
	server1 192.168.0.10
	sync-attributes email=mail
	sync-defaults-options enable-new=0,remove-vanished=acl;entry
	user-attr uid
	user-classes inetorgperson,posixaccount,person,user


You can use the ``proxmox-backup-manager openid`` and ``proxmox-backup-manager ldap`` commands to manipulate
this file.
