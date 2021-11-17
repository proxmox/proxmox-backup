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
	    xtype: 'pmxTfaView',
	    title: gettext('Two Factor Authentication'),
	    itemId: 'tfa',
	    iconCls: 'fa fa-key',
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
	    baseUrl: '/config/access',
	    useTypeInUrl: true,
	    title: gettext('Realms'),
	    itemId: 'domains',
	    iconCls: 'fa fa-address-book-o',
	},
    ],

});
