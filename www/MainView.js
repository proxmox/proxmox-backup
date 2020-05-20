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
		    ':path'    : '(?:([%a-zA-Z0-9\\-\\_\\s,\.]+))',
		    ':subpath' : '(?:(?::)([%a-zA-Z0-9\\-\\_\\s,]+))?'
		}
	    }
	},

	beforeChangePath: function(path, subpath, action) {
	    var me = this;

	    let xtype = path;
	    let datastore;
	    let isDataStore = PBS.Utils.isDataStorePath(path);
	    if (isDataStore) {
		xtype = 'pbsDataStorePanel';
		datastore = PBS.Utils.getDataStoreFromPath(path);
	    }

	    if (!Ext.ClassManager.getByAlias(`widget.${xtype}`)) {
		console.warn(`xtype ${xtype} not found`);
		action.stop();
		return;
	    }

	    var lastpanel = me.lookupReference('contentpanel').getLayout().getActiveItem();
	    if (lastpanel && lastpanel.xtype === xtype) {
		if (isDataStore) {
		    if (datastore === lastpanel.datastore) {
			action.stop();
			return;
		    }
		} else {
		    // we have the right component already,
		    // we just need to select the correct tab
		    // default to the first
		    subpath = subpath || 0;
		    if (lastpanel.getActiveTab) {
			// we assume lastpanel is a tabpanel
			if (lastpanel.getActiveTab().getItemId() !== subpath) {
			    // set the active tab
			    lastpanel.setActiveTab(subpath);
			}
			// else we are already there
		    }
		    action.stop();
		    return;
		}
	    }

	    action.resume();
	},

	changePath: function(path, subpath) {
	    var me = this;
	    var contentpanel = me.lookupReference('contentpanel');
	    var lastpanel = contentpanel.getLayout().getActiveItem();

	    var obj;
	    if (PBS.Utils.isDataStorePath(path)) {
		let datastore = PBS.Utils.getDataStoreFromPath(path);
		obj = contentpanel.add({
		    xtype: 'pbsDataStorePanel',
		    datastore,
		});
	    } else {
		obj = contentpanel.add({ xtype: path, border: false });
	    }

	    var treelist = me.lookupReference('navtree');

	    treelist.suspendEvents();
	    if (subpath === undefined) {
		treelist.select(path);
	    } else {
		treelist.select(path + ':' + subpath);
	    }
	    treelist.resumeEvents();

	    if (Ext.isFunction(obj.setActiveTab)) {
		obj.setActiveTab(subpath || 0);
		obj.addListener('tabchange', function(tabpanel, newc, oldc) {
		    var newpath = path;

		    // only add the subpath part for the
		    // non-default tabs
		    if (tabpanel.items.findIndex('id', newc.id) !== 0) {
			newpath += ":" + newc.getItemId();
		    }

		    me.redirectTo(newpath);
		});
	    }

	    contentpanel.setActiveItem(obj);

	    if (lastpanel) {
		contentpanel.remove(lastpanel, { destroy: true });
	    }

	},

	logout: function() {
	    PBS.app.logout();
	},

	navigate: function(treelist, item) {
	    this.redirectTo(item.get('path'));
	},

	control: {
	    'button[reference=logoutButton]': {
		click: 'logout'
	    }
	},

	init: function(view) {
	    var me = this;

	    me.lookupReference('usernameinfo').update({username:Proxmox.UserName});

	    // show login on requestexception
	    // fixme: what about other errors
	    Ext.Ajax.on('requestexception', function(conn, response, options) {
		if (response.status == 401) { // auth failure
		    me.logout();
		}
	    });

	    // get ticket periodically
	    Ext.TaskManager.start({
		run: function() {
		    var ticket = Proxmox.Utils.authOK();
		    if (!ticket || !Proxmox.UserName) {
			return;
		    }

		    Ext.Ajax.request({
			params: {
			    username: Proxmox.UserName,
			    password: ticket
			},
			url: '/api2/json/access/ticket',
			method: 'POST',
			failure: function() {
			    me.logout();
			},
			success: function(response, opts) {
			    var obj = Ext.decode(response.responseText);
			    PBS.Utils.updateLoginData(obj.data);
			}
		    });
		},
		interval: 15*60*1000
	    });


	    // select treeitem and load page from url fragment, if set
	    let token = Ext.util.History.getToken() || 'pbsDashboard';
	    this.redirectTo(token, true);
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
		    xtype: 'proxmoxlogo',
		    prefix: '',
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
		    xtype: 'button',
		    baseCls: 'x-btn',
		    cls: 'x-btn-default-toolbar-small proxmox-inline-button',
		    iconCls: 'fa fa-book x-btn-icon-el-default-toolbar-small ',
		    text: gettext('Documentation'),
		    href: '/docs/index.html',
		    margin: '0 5 0 0',
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
	    items: [{
		xtype: 'navigationtree',
		minWidth: 180,
		reference: 'navtree',
		// we have to define it here until extjs 6.2
		// because of a bug where a viewcontroller does not detect
		// the selectionchange event of a treelist
		listeners: {
		    selectionchange: 'navigate'
		}
	    }, {
		xtype: 'box',
		cls: 'x-treelist-nav',
		flex: 1
	    }]
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
