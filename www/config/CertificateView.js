Ext.define('PBS.config.CertificateConfiguration', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsCertificateConfiguration',

    title: gettext('Certificates'),

    border: false,
    defaults: { border: false },

    items: [
       {
           itemId: 'certificates',
           xtype: 'pbsCertificatesView',
       },
       {
           itemId: 'acme',
           xtype: 'pbsACMEConfigView',
       },
    ],
});

Ext.define('PBS.config.CertificatesView', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsCertificatesView',

    title: gettext('Certificates'),
    border: false,
    defaults: {
	border: false,
    },
    scrollable: 'y',

    items: [
	{
	    xtype: 'pmxCertificates',
	    nodename: 'localhost',
	    infoUrl: '/nodes/localhost/certificates/info',
	    uploadButtons: [
		{
		    id: 'proxy.pem',
		    url: '/nodes/localhost/certificates/custom',
		    deletable: true,
		    reloadUi: true,
		},
	    ],
	},
	{
	    xtype: 'pmxACMEDomains',
	    border: 0,
	    url: `/nodes/localhost/config`,
	    nodename: 'localhost',
	    acmeUrl: '/config/acme',
	    orderUrl: `/nodes/localhost/certificates/acme/certificate`,
	    separateDomainEntries: true,
	},
    ],
});

Ext.define('PBS.ACMEConfigView', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsACMEConfigView',

    title: gettext('ACME Accounts'),

    //onlineHelp: 'sysadmin_certificate_management',

    items: [
       {
           region: 'north',
           border: false,
           xtype: 'pmxACMEAccounts',
           acmeUrl: '/config/acme',
       },
       {
           region: 'center',
           border: false,
           xtype: 'pmxACMEPluginView',
           acmeUrl: '/config/acme',
       },
    ],
});
