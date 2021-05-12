Ext.define('PBS.SystemConfiguration', {
    extend: 'Ext.tab.Panel',
    xtype: 'pbsSystemConfiguration',

    title: gettext('Configuration') + ': ' + gettext('System'),
    border: true,
    scrollable: true,
    defaults: { border: false },
    tools: [PBS.Utils.get_help_tool("sysadmin-network-configuration")],
    items: [
	{
	    title: gettext('Network/Time'),
	    itemId: 'network',
	    xtype: 'panel',
	    layout: {
		type: 'vbox',
		align: 'stretch',
		multi: true,
	    },
	    defaults: {
		collapsible: true,
		animCollapse: false,
		margin: '10 10 0 10',
	    },
	    items: [
		{
		    title: gettext('Time'),
		    xtype: 'proxmoxNodeTimeView',
		    nodename: 'localhost',
		},
		{
		    title: gettext('DNS'),
		    xtype: 'proxmoxNodeDNSView',
		    nodename: 'localhost',
		},
		{
		    flex: 1,
		    minHeight: 200,
		    title: gettext('Network Interfaces'),
		    xtype: 'proxmoxNodeNetworkView',
		    showApplyBtn: true,
		    types: ['bond', 'bridge'],
		    nodename: 'localhost',
		},
	    ],
	},
	{
	    title: gettext('Authentication'),
	    itemId: 'authentication',
	    xtype: 'panel',
	    layout: {
		type: 'vbox',
		align: 'stretch',
		multi: true,
	    },
	    defaults: {
		collapsible: true,
		animCollapse: false,
		margin: '10 10 0 10',
	    },
	    items: [
		{
		    title: gettext('Webauthn'),
		    xtype: 'pbsWebauthnConfigView',
		},
	    ],
	},
	{
	    title: gettext('Options'),
	    itemId: 'options',
	    xtype: 'pbsNodeOptionView',
	},
    ],

    initComponent: function() {
	let me = this;

	me.callParent();

	let networktime = me.getComponent('network');
	Ext.Array.forEach(networktime.query(), function(item) {
	    item.relayEvents(networktime, ['activate', 'deactivate', 'destroy']);
	});

	let authentication = me.getComponent('authentication');
	Ext.Array.forEach(authentication.query(), function(item) {
	    item.relayEvents(authentication, ['activate', 'deactivate', 'destroy']);
	});
    },
});


