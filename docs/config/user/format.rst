This file contains the list of API users and API tokens.

Each user configuration section starts with the header ``user: <name>``,
followed by the user configuration options.

API token configuration starts with the header ``token:
<userid!token_name>``, followed by the token configuration. The data
used to authenticate tokens is stored in a separate file
(``token.shadow``).


::

  user: root@pam
	comment Superuser
	email test@example.local
	...

  token: root@pam!token1
	comment API test token
	enable true
	expire 0

  user: ...


You can use the ``proxmox-backup-manager user`` command to manipulate
this file.
