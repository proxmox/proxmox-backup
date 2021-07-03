Ext.define('PBS.view.main.VersionInfo', {
    extend: 'Ext.Component',
    xtype: 'versioninfo',

    makeApiCall: true,

    data: {
	version: false,
    },

    style: {
	'font-size': '14px',
	'line-height': '18px',
    },

    tpl: [
	'Backup Server',
	'<tpl if="version">',
	' {version}-{release}',
	'</tpl>',
    ],

    initComponent: function() {
	var me = this;
	me.callParent();

	if (me.makeApiCall) {
	    Proxmox.Utils.API2Request({
		url: '/version',
		method: 'GET',
		success: function(response) {
		    me.update(response.result.data);
		},
	    });
	}
    },
});
