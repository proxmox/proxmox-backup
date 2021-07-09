Ext.define('PBS.AccessControlPanel', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsAccessControlPanel',
    mixins: ['Proxmox.Mixin.CBind'],

    title: gettext('Access Control'),

    tools: [PBS.Utils.get_help_tool("user-mgmt")],

    border: false,
    defaults: {
	border: false,
    },

    items: [
	{
	    xtype: 'pbsUserView',
	    title: gettext('User Management'),
	    itemId: 'users',
	    iconCls: 'fa fa-user',
	},
	{
	    xtype: 'pbsTfaView',
	    title: gettext('Two Factor Authentication'),
	    itemId: 'tfa',
	    iconCls: 'fa fa-id-badge',
	},
	{
	    xtype: 'pbsTokenView',
	    title: gettext('API Token'),
	    itemId: 'apitokens',
	    iconCls: 'fa fa-user-o',
	},
	{
	    xtype: 'pbsACLView',
	    title: gettext('Permissions'),
	    itemId: 'permissions',
	    iconCls: 'fa fa-unlock',
	},
	{
	    xtype: 'pmxAuthView',
	    title: gettext('Authentication'),
	    itemId: 'domains',
	    iconCls: 'fa fa-key',
	},
    ],

});
