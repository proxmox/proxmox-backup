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
                conditions: {
		    ':path': '(?:([%a-zA-Z0-9\\-\\_\\s,.]+))',
		    ':subpath': '(?:(?::)([%a-zA-Z0-9\\-\\_\\s,]+))?',
		},
	    },
	},

	parseRouterPath: function(path) {
	    let xtype = path;
	    let config = {};
	    if (PBS.Utils.isDataStorePath(path)) {
		config.datastore = PBS.Utils.getDataStoreFromPath(path);
		xtype = 'pbsDataStorePanel';
	    } else if (path.indexOf('Changer-') === 0) {
		config.changer = path.slice('Changer-'.length);
		xtype = 'pbsChangerStatus';
	    } else if (path.indexOf('Drive-') === 0) {
		config.drive = path.slice('Drive-'.length);
		xtype = 'pbsDriveStatus';
	    }

	    return [xtype, config];
	},

	beforeChangePath: function(path, subpathOrAction, action) {
	    var me = this;

	    let subpath = subpathOrAction;
	    if (!action) {
		action = subpathOrAction;
		subpath = undefined;
	    }

	    let [xtype, config] = me.parseRouterPath(path);

	    if (!Ext.ClassManager.getByAlias(`widget.${xtype}`)) {
		console.warn(`xtype ${xtype} not found`);
		action.stop();
		return;
	    }

	    var lastpanel = me.lookupReference('contentpanel').getLayout().getActiveItem();
	    if (lastpanel && lastpanel.xtype === xtype) {
		for (const [prop, value] of Object.entries(config)) {
		    if (lastpanel[prop] !== value) {
			action.resume();
			return;
		    }
		}
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

	    action.resume();
	},

	changePath: function(path, subpath) {
	    var me = this;
	    var contentpanel = me.lookupReference('contentpanel');
	    var lastpanel = contentpanel.getLayout().getActiveItem();

	    let tabChangeListener = function(tp, newc, oldc) {
		let newpath = path;

		// only add the subpath part for the
		// non-default tabs
		if (tp.items.findIndex('id', newc.id) !== 0) {
		    newpath += `:${newc.getItemId()}`;
		}

		me.redirectTo(newpath);
	    };

	    let [xtype, config] = me.parseRouterPath(path);
	    var obj;
	    if (PBS.Utils.isDataStorePath(path)) {
		if (lastpanel && lastpanel.xtype === xtype && !subpath) {
		    let activeTab = lastpanel.getActiveTab();
		    let newpath = path;
		    if (lastpanel.items.indexOf(activeTab) !== 0) {
			subpath = activeTab.getItemId();
			newpath += `:${subpath}`;
		    }
		    me.redirectTo(newpath);
		}
	    }
	    obj = contentpanel.add(Ext.apply(config, {
		xtype,
		nodename: 'localhost',
		border: false,
		activeTab: subpath || 0,
		listeners: {
		    tabchange: tabChangeListener,
		},
	    }));

	    var treelist = me.lookupReference('navtree');

	    treelist.select(path, true);

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
	    '[reference=logoutButton]': {
		click: 'logout',
	    },
	},

	init: function(view) {
	    var me = this;

	    PBS.data.RunningTasksStore.startUpdate();
	    me.lookupReference('usernameinfo').setText(Proxmox.UserName);

	    // show login on requestexception
	    // fixme: what about other errors
	    Ext.Ajax.on('requestexception', function(conn, response, options) {
		if (response.status === 401 || response.status === '401') { // auth failure
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
			    password: ticket,
			},
			url: '/api2/json/access/ticket',
			method: 'POST',
			failure: function() {
			    me.logout();
			},
			success: function(response, opts) {
			    var obj = Ext.decode(response.responseText);
			    PBS.Utils.updateLoginData(obj.data);
			},
		    });
		},
		interval: 15*60*1000,
	    });

	    Proxmox.Utils.API2Request({
		url: `/api2/extjs/nodes/localhost/status`,
		success: function({ result }) {
		    if (result?.data?.info?.fingerprint) {
			Proxmox.Fingerprint = result.data.info.fingerprint;
		    }
		},
		failure: function() {
		    // silently ignore errors
		},
	    });

	    // select treeitem and load page from url fragment, if set
	    let token = Ext.util.History.getToken() || 'pbsDashboard';
	    this.redirectTo(token, { force: true });
	},
    },

    plugins: 'viewport',

    layout: { type: 'border' },

    items: [
	{
	    region: 'north',
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'middle',
	    },
	    margin: '2 0 2 5',
	    height: 38,
	    items: [
		{
		    xtype: 'proxmoxlogo',
		    prefix: '',
		},
		{
		    padding: '0 0 0 5',
		    xtype: 'versioninfo',
		},
		{
		    flex: 1,
		    baseCls: 'x-plain',
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
		    xtype: 'pbsTaskButton',
		    margin: '0 5 0 0',
		},
		{
		    xtype: 'button',
		    reference: 'usernameinfo',
		    style: {
			// proxmox dark grey p light grey as border
			backgroundColor: '#464d4d',
			borderColor: '#ABBABA',
		    },
		    margin: '0 5 0 0',
		    iconCls: 'fa fa-user',
		    menu: [
			{
			    iconCls: 'fa fa-gear',
			    text: gettext('My Settings'),
			    handler: () => Ext.create('PBS.window.Settings').show(),
			},
			{
			    iconCls: 'fa fa-paint-brush',
			    text: gettext('Color Theme'),
			    handler: () => Ext.create('Proxmox.window.ThemeEditWindow', {
				cookieName: 'PBSThemeCookie',
				autoShow: true,
			    }),
			},
			{
			    iconCls: 'fa fa-language',
			    text: gettext('Language'),
			    reference: 'languageButton',
			    handler: () => Ext.create('Proxmox.window.LanguageEditWindow', {
				cookieName: 'PBSLangCookie',
				autoShow: true,
			    }),
			},
			'-',
			{
			    iconCls: 'fa fa-sign-out',
			    text: gettext('Logout'),
			    reference: 'logoutButton',
			},
		    ],
		},
	    ],
	},
	{
	    xtype: 'container',
	    scrollable: 'y',
	    border: false,
	    region: 'west',
	    layout: {
		type: 'vbox',
		align: 'stretch',
	    },
	    items: [{
		xtype: 'navigationtree',
		minWidth: 180,
		ui: 'pve-nav',
		reference: 'navtree',
		// we have to define it here until extjs 6.2
		// because of a bug where a viewcontroller does not detect
		// the selectionchange event of a treelist
		listeners: {
		    selectionchange: 'navigate',
		},
	    }, {
		xtype: 'box',
		cls: 'x-treelist-pve-nav',
		flex: 1,
	    }],
	},
	{
	    xtype: 'container',
	    layout: { type: 'card' },
	    region: 'center',
	    border: false,
	    reference: 'contentpanel',
	},
    ],
});
