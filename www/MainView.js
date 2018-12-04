Ext.define('PBS.MainView', {
    extend: 'Ext.container.Container',
    xtype: 'mainview',

    title: 'Proxmox Backup Server',

    controller: {
	xclass: 'Ext.app.ViewController',
	routes: {
	    ':path:subpath': {
		action: 'changePath',
		before: 'beforeChangePath',
                conditions : {
		    ':path'    : '(?:([%a-zA-Z0-9\\-\\_\\s,]+))',
		    ':subpath' : '(?:(?::)([%a-zA-Z0-9\\-\\_\\s,]+))?'
		}
	    }
	},
	
	beforeChangePath: function(path, subpath, action) {
	    var me = this;

	    action.resume();
	},
	
       	changePath: function(path,subpath) {
	    var me = this;
	    var contentpanel = me.lookupReference('contentpanel');
	    var lastpanel = contentpanel.getLayout().getActiveItem();

	},

	init: function(view) {
	    var me = this;
	    console.log("init");

	}
    },

    plugins: 'viewport',

    layout: { type: 'border' },

    items: [
	{
	    region: 'north',
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'middle'
	    },
	    margin: '2 5 2 5',
	    height: 38,
	    items: [
		{
		    xtype: 'proxmoxlogo'
		},
		{
		    xtype: 'versioninfo'
		},
		{
		    flex: 1
		},
		{
		    baseCls: 'x-plain',
		    reference: 'usernameinfo',
		    padding: '0 5',
		    tpl: Ext.String.format(gettext("You are logged in as {0}"), "'{username}'")
		},
		{
		    reference: 'logoutButton',
		    xtype: 'button',
		    iconCls: 'fa fa-sign-out',
		    text: gettext('Logout')
		}
	    ]
	},
	{
	    xtype: 'panel',
	    scrollable: 'y',
	    border: false,
	    region: 'west',
	    layout: {
		type: 'vbox',
		align: 'stretch'
	    },
	    items: [{ html: "test" }]
	},
	{
	    xtype: 'panel',
	    layout: { type: 'card' },
	    region: 'center',
	    border: false,
	    reference: 'contentpanel'
	}
    ]
});

 
