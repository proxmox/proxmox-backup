/*global Proxmox*/
Ext.define('PBS.view.main.VersionInfo',{
    extend: 'Ext.Component',
    xtype: 'versioninfo',

    makeApiCall: true,

    data: {
	version: false
    },

    tpl: [
	'Backup Server',
	'<tpl if="version">',
	' {version}-{release}',
	'</tpl>'
    ],

    initComponent: function() {
	var me = this;
	me.callParent();

	if (me.makeApiCall) {
	    PBS.Utils.API3Request({
		url: '/version',
		method: 'GET',
		success: function(response) {
		    me.update(response.result.data);
		}
	    });
	}
    }
});
