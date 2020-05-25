/*global Proxmox*/

Ext.define('PBS.SystemConfiguration', {
    extend: 'Ext.tab.Panel',
    xtype: 'pbsSystemConfiguration',

    title: gettext('Configuration') + ': ' + gettext('System'),
    border: true,
    scrollable: true,
    defaults: { border: false },
    items: [
	{
	    title: gettext('Network/Time'),
	    itemId: 'network',
	    xtype: 'panel',
	    layout: {
		type: 'vbox',
		align: 'stretch',
		multi: true
	    },
	    defaults: {
		collapsible: true,
		animCollapse: false,
		margin: '10 10 0 10'
	    },
	    items: [
		{
		    title: gettext('Time'),
		    xtype: 'proxmoxNodeTimeView',
		    nodename: 'localhost'
		},
		{
		    title: gettext('DNS'),
		    xtype: 'proxmoxNodeDNSView',
		    nodename: 'localhost'
		},
		{
		    flex: 1,
		    minHeight: 200,
		    title: gettext('Network Interfaces'),
		    xtype: 'proxmoxNodeNetworkView',
		    showApplyBtn: true,
		    types: ['bond', 'bridge', 'vlan'],
		    nodename: 'localhost'
		},
	    ]
//	},
//	{
//	    itemId: 'options',
//          title: gettext('Options'),
//	    html: "TESWT"
//	    xtype: 'pbsSystemOptions'
	}
    ],

    initComponent: function() {
	var me = this;

	me.callParent();

	var networktime = me.getComponent('network');
	Ext.Array.forEach(networktime.query(), function(item) {
	    item.relayEvents(networktime, [ 'activate', 'deactivate', 'destroy']);
	});
    }
});


